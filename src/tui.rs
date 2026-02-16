// src/tui.rs
//
// =============================================================================
// UNIFIEDLAB: DASHBOARD (v 0.1 )
// =============================================================================
//
// The Visual Control Center.
//
// Features:
// 1. Cluster Metrics (Cores, Throughput).
// 2. Job Table (Filterable by Engine/Status).
// 3. Deep Inspector (Engine-specific details & Provenance).
// 4. Real-time Log Stream.
//
// TODO:
//   general usability improvements
//   at some point post processing module implementation?

use crate::checkpoint::{CheckpointStore, WorkerInfo};
use crate::core::{ElectronVolts, Engine, Job, JobStatus, JobSummary};
use crate::logs::LogBuffer;
use crate::resources::SystemMonitor;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Clear, Gauge, List, ListItem, Paragraph, Row, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Sparkline, Table, TableState, Tabs, Wrap,
    },
    Frame,
};
use std::{
    io,
    path::PathBuf,
    time::{Duration, Instant},
};

// --- Metrics Snapshot ---
#[derive(Default)]
struct ClusterMetrics {
    total_jobs: usize,
    running: usize,
    completed: usize,
    failed: usize,
    pending: usize,

    // Hardware
    cores_allocated: usize,
    cores_total: usize,
    capacity_percent: f64,
    throughput_history: Vec<u64>,
}

pub struct TuiApp {
    ckpt_path: PathBuf,
    store: Option<CheckpointStore>,
    log_buffer: LogBuffer,

    // Data
    jobs_summary: Vec<JobSummary>,
    visible_jobs: Vec<JobSummary>,
    workers: Vec<WorkerInfo>,

    // UI State
    table_state: TableState,
    scrollbar_state: ScrollbarState,
    current_tab: usize,
    selected_job_id: String,
    inspector_lines: Vec<Line<'static>>,

    should_quit: bool,
    show_help: bool,
    status_msg: String,
    status_color: Color,
    cluster_info: String,

    last_refresh: Instant,
    refresh_period: Duration,
    metrics: ClusterMetrics,
}

impl TuiApp {
    pub fn new(ckpt_path: &str, log_buffer: LogBuffer) -> Self {
        let mut sys = SystemMonitor::new();
        let env = sys.snapshot();
        let cluster_info = format!("{:?} ({})", env.cluster_type, env.hostname);

        Self {
            ckpt_path: PathBuf::from(ckpt_path),
            store: None,
            log_buffer,
            jobs_summary: Vec::new(),
            visible_jobs: Vec::new(),
            workers: Vec::new(),
            table_state: TableState::default(),
            scrollbar_state: ScrollbarState::default(),
            current_tab: 0,
            selected_job_id: String::new(),
            inspector_lines: vec![Line::from("Select a node to inspect payload")],
            should_quit: false,
            show_help: false,
            status_msg: "Init".into(),
            status_color: Color::Gray,
            cluster_info,
            last_refresh: Instant::now(),
            refresh_period: Duration::from_millis(500),
            metrics: ClusterMetrics::default(),
        }
    }

    pub fn run(&mut self) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        self.refresh_data();

        while !self.should_quit {
            if self.last_refresh.elapsed() >= self.refresh_period {
                self.refresh_data();
                self.last_refresh = Instant::now();
            }

            terminal.draw(|f| self.ui(f))?;

            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    self.handle_input(key);
                }
            }
        }

        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        Ok(())
    }

    // --- Data Management ---

    fn refresh_data(&mut self) {
        // 1. Connect (Lazy)
        if self.store.is_none() {
            if self.ckpt_path.exists() {
                match CheckpointStore::open(&self.ckpt_path) {
                    Ok(s) => {
                        self.store = Some(s);
                        self.status_msg = "ONLINE".into();
                        self.status_color = Color::Green;
                    }
                    Err(_) => {
                        self.status_msg = "DB LOCK".into();
                        self.status_color = Color::Red;
                        return;
                    }
                }
            } else {
                self.status_msg = "WAITING".into();
                self.status_color = Color::Yellow;
                return;
            }
        }

        // 2. Fetch
        let (fetched_workers, fetched_jobs) = if let Some(store) = &self.store {
            (
                store.get_active_workers().ok(),
                store.get_jobs_summary().ok(),
            )
        } else {
            (None, None)
        };

        // 3. Update
        if let Some(w) = fetched_workers {
            self.workers = w;
        }
        if let Some(j) = fetched_jobs {
            self.jobs_summary = j;
            self.recalc_metrics();
            self.apply_tab_filter();
        }

        // 4. Inspect Detail
        let mut id_to_fetch = None;
        if let Some(idx) = self.table_state.selected() {
            if idx < self.visible_jobs.len() {
                let current = &self.visible_jobs[idx];
                if current.id != self.selected_job_id
                    || current.status == "Running"
                    || current.status == "Pending"
                {
                    self.selected_job_id = current.id.clone();
                    id_to_fetch = Some(self.selected_job_id.clone());
                }
            }
        }

        if let Some(id) = id_to_fetch {
            if let Some(store) = &self.store {
                if let Ok(job) = store.get_job_details(&id) {
                    self.inspector_lines = Self::format_inspector(&job);
                }
            }
        }
    }

    fn recalc_metrics(&mut self) {
        let m = &mut self.metrics;
        m.total_jobs = self.jobs_summary.len();
        m.running = 0;
        m.completed = 0;
        m.failed = 0;
        m.pending = 0;

        for j in &self.jobs_summary {
            match j.status.as_str() {
                "Running" => m.running += 1,
                "Completed" => m.completed += 1,
                "Failed" => m.failed += 1,
                "Pending" | "Blocked" => m.pending += 1,
                _ => {}
            }
        }

        if m.throughput_history.len() >= 60 {
            m.throughput_history.remove(0);
        }
        m.throughput_history.push(m.running as u64);

        let active_nodes: Vec<&WorkerInfo> = self.workers.iter().filter(|w| w.cores > 0).collect();
        m.cores_allocated = active_nodes.iter().map(|w| w.tasks).sum(); // Approx: 1 task != 1 core, but decent proxy
        m.cores_total = active_nodes.iter().map(|w| w.cores).sum();

        m.capacity_percent = if m.cores_total > 0 {
            (m.cores_allocated as f64 / m.cores_total as f64).min(1.0)
        } else {
            0.0
        };
    }

    fn apply_tab_filter(&mut self) {
        self.visible_jobs = self
            .jobs_summary
            .iter()
            .filter(|j| match self.current_tab {
                0 => true,
                1 => matches!(j.status.as_str(), "Pending" | "Running" | "Blocked"),
                2 => j.status == "Completed",
                3 => j.status == "Failed",
                4 => j.code.contains("agent"),
                _ => true,
            })
            .cloned()
            .collect();

        self.scrollbar_state = self.scrollbar_state.content_length(self.visible_jobs.len());
    }

    // --- UI Layout ---

    fn ui(&mut self, f: &mut Frame) {
        let area = f.area();
        let layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(20),
                Constraint::Percentage(50),
                Constraint::Percentage(30),
            ])
            .split(area);

        self.draw_sidebar(f, layout[0]);
        self.draw_main(f, layout[1]);
        self.draw_inspector(f, layout[2]);

        if self.show_help {
            self.draw_help(f);
        }
    }

    fn draw_sidebar(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(10),
                Constraint::Length(3),
                Constraint::Length(6),
                Constraint::Min(0),
            ])
            .split(area);

        let info_text = vec![
            Line::from(Span::styled(
                " UNIFIEDLAB v6 ",
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(vec![
                Span::raw("Env:   "),
                Span::styled(&self.cluster_info, Style::default().fg(Color::Cyan)),
            ]),
            Line::from(vec![
                Span::raw("DB:    "),
                Span::styled(&self.status_msg, Style::default().fg(self.status_color)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::raw("Total: "),
                Span::styled(
                    self.metrics.total_jobs.to_string(),
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from(vec![
                Span::raw("Run:   "),
                Span::styled(
                    self.metrics.running.to_string(),
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::raw("Done:  "),
                Span::styled(
                    self.metrics.completed.to_string(),
                    Style::default().fg(Color::Green),
                ),
            ]),
            Line::from(vec![
                Span::raw("Fail:  "),
                Span::styled(
                    self.metrics.failed.to_string(),
                    Style::default().fg(Color::Red),
                ),
            ]),
        ];
        f.render_widget(
            Paragraph::new(info_text).block(Block::default().borders(Borders::ALL)),
            chunks[0],
        );

        let gauge = Gauge::default()
            .block(Block::default().borders(Borders::ALL).title("Core Load"))
            .gauge_style(Style::default().fg(if self.metrics.capacity_percent > 0.9 {
                Color::Red
            } else {
                Color::Green
            }))
            .ratio(self.metrics.capacity_percent)
            .label(format!(
                "{}/{}",
                self.metrics.cores_allocated, self.metrics.cores_total
            ));
        f.render_widget(gauge, chunks[1]);

        let spark = Sparkline::default()
            .block(Block::default().borders(Borders::ALL).title("Throughput"))
            .data(&self.metrics.throughput_history)
            .style(Style::default().fg(Color::Magenta));
        f.render_widget(spark, chunks[2]);

        let node_list: Vec<ListItem> = self
            .workers
            .iter()
            .filter(|w| !w.worker_id.contains("submitter") && !w.worker_id.contains("architect"))
            .map(|w| {
                let load = if w.cores > 0 {
                    w.tasks as f64 / w.cores as f64
                } else {
                    0.0
                };
                let color = if load > 0.8 {
                    Color::Red
                } else if load > 0.0 {
                    Color::Green
                } else {
                    Color::Gray
                };
                let short_id = w.worker_id.split('_').next().unwrap_or("?");
                ListItem::new(format!("{} [{}]", short_id, w.tasks))
                    .style(Style::default().fg(color))
            })
            .collect();
        f.render_widget(
            List::new(node_list).block(Block::default().borders(Borders::ALL).title("Guardians")),
            chunks[3],
        );
    }

    fn draw_main(&mut self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(7),
            ])
            .split(area);

        let tabs = Tabs::new(vec![" ALL ", " ACTIVE ", " DONE ", " FAILED ", " AGENTS "])
            .block(Block::default().borders(Borders::ALL))
            .select(self.current_tab)
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
            .divider("|");
        f.render_widget(tabs, chunks[0]);

        let rows: Vec<Row> = self
            .visible_jobs
            .iter()
            .map(|j| {
                let (icon, color) = match j.status.as_str() {
                    "Running" => ("▶", Color::Yellow),
                    "Completed" => ("✔", Color::Green),
                    "Failed" => ("✖", Color::Red),
                    "Blocked" => ("⏸", Color::Magenta),
                    "Pending" => ("●", Color::Blue),
                    _ => ("?", Color::DarkGray),
                };

                Row::new(vec![
                    Cell::from(j.id.chars().take(8).collect::<String>()),
                    Cell::from(format!("{} {}", icon, j.status)).style(Style::default().fg(color)),
                    Cell::from(j.code.clone()),
                    Cell::from(format!("{:.0}ms", j.t_total)),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(10),
                Constraint::Length(12),
                Constraint::Min(15),
                Constraint::Length(10),
            ],
        )
        .header(
            Row::new(vec!["ID", "Status", "Engine", "Time"])
                .style(Style::default().fg(Color::Cyan)),
        )
        .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
        .row_highlight_style(Style::default().bg(Color::Rgb(40, 40, 40)));

        f.render_stateful_widget(table, chunks[1], &mut self.table_state);
        f.render_stateful_widget(
            Scrollbar::default().orientation(ScrollbarOrientation::VerticalRight),
            chunks[1],
            &mut self.scrollbar_state,
        );

        let logs = self.log_buffer.get_lines();
        let log_list = List::new(
            logs.iter()
                .rev()
                .take(6)
                .map(|s| ListItem::new(format!("> {}", s)))
                .collect::<Vec<_>>(),
        )
        .block(Block::default().borders(Borders::TOP).title("Events"));
        f.render_widget(log_list, chunks[2]);
    }

    fn draw_inspector(&self, f: &mut Frame, area: Rect) {
        let block = Block::default().borders(Borders::ALL).title(" Inspector ");
        f.render_widget(
            Paragraph::new(self.inspector_lines.clone())
                .block(block)
                .wrap(Wrap { trim: true }),
            area,
        );
    }

    fn format_inspector(job: &Job) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let status_style = match job.status {
            JobStatus::Running => Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            JobStatus::Completed => Style::default().fg(Color::Green),
            JobStatus::Failed => Style::default().fg(Color::Red),
            _ => Style::default().fg(Color::White),
        };

        // FIXED: Clone strings to own data for 'static lifetime
        lines.push(Line::from(vec![
            Span::styled("ID: ", Style::default().fg(Color::Cyan)),
            Span::raw(job.id.to_string()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("St: ", Style::default().fg(Color::Cyan)),
            Span::styled(format!("{:?}", job.status), status_style),
        ]));
        if let Some(node) = &job.node_id {
            lines.push(Line::from(vec![
                Span::styled("Guardian: ", Style::default().fg(Color::Yellow)),
                Span::raw(node.clone()),
            ]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " ENGINE CONFIG ",
            Style::default().bg(Color::DarkGray),
        )));

        match &job.config.engine {
            Engine::Janus {
                arch,
                device_preference,
                ..
            } => {
                lines.push(Line::from(vec![
                    Span::raw("Type: "),
                    Span::styled("Janus (MLIP)", Style::default().fg(Color::Magenta)),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("Arch: "),
                    Span::raw(arch.clone()),
                ]));
                if let Some(dev) = device_preference {
                    lines.push(Line::from(vec![
                        Span::raw("Device: "),
                        Span::raw(dev.clone()),
                    ]));
                }
            }
            Engine::Gulp { binary, .. } => {
                lines.push(Line::from(vec![
                    Span::raw("Type: "),
                    Span::styled("GULP", Style::default().fg(Color::Blue)),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("Bin:  "),
                    Span::raw(binary.clone()),
                ]));
            }
            Engine::Vasp { mpi_ranks, .. } => {
                lines.push(Line::from(vec![
                    Span::raw("Type: "),
                    Span::styled("VASP", Style::default().fg(Color::Yellow)),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("Ranks: "),
                    Span::raw(mpi_ranks.to_string()),
                ]));
            }
            Engine::Cp2k { mpi_ranks, .. } => {
                lines.push(Line::from(vec![
                    Span::raw("Type: "),
                    Span::styled("CP2K", Style::default().fg(Color::LightBlue)),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("Ranks: "),
                    Span::raw(mpi_ranks.to_string()),
                ]));
            }
            Engine::Agent { strategy, .. } => {
                lines.push(Line::from(vec![
                    Span::raw("Type: "),
                    Span::styled("Agent", Style::default().fg(Color::Green)),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("Strat: "),
                    Span::raw(strategy.clone()),
                ]));
            }
        }

        if let Some(res) = &job.result {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " RESULT & PROVENANCE ",
                Style::default().bg(Color::DarkGray),
            )));

            if let Some(ElectronVolts(ev)) = res.energy {
                lines.push(Line::from(vec![
                    Span::raw("Energy: "),
                    Span::styled(format!("{:.4} eV", ev), Style::default().fg(Color::Green)),
                ]));
            }
            lines.push(Line::from(vec![
                Span::raw("Time:   "),
                Span::styled(
                    format!("{:.1}ms", res.t_total_ms),
                    Style::default().fg(Color::Cyan),
                ),
            ]));

            lines.push(Line::from(vec![
                Span::raw("Host:   "),
                Span::raw(res.provenance.execution_host.clone()),
            ]));
            lines.push(Line::from(vec![
                Span::raw("Sandbox: "),
                Span::styled(
                    res.provenance.sandbox_info.clone(),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));

            if let Some(h) = &res.provenance.binary_hash {
                let h_clone = h.clone();
                let short = if h_clone.len() > 8 {
                    &h_clone[0..8]
                } else {
                    &h_clone
                };
                lines.push(Line::from(vec![
                    Span::raw("BinHash: "),
                    Span::raw(short.to_string()),
                ]));
            }
        }

        if let Some(err) = &job.error_log {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " ERROR ",
                Style::default().bg(Color::Red),
            )));
            for l in err.lines().take(5) {
                lines.push(Line::from(Span::styled(
                    l.to_string(),
                    Style::default().fg(Color::Red),
                )));
            }
        }

        lines
    }

    fn handle_input(&mut self, key: event::KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        if self.show_help {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?')
            ) {
                self.show_help = false;
            }
            return;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char('r') => self.refresh_data(),
            KeyCode::Tab => {
                self.current_tab = (self.current_tab + 1) % 5;
                self.table_state.select(Some(0));
                self.refresh_data();
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            _ => {}
        }
    }

    fn move_selection(&mut self, delta: i32) {
        if self.visible_jobs.is_empty() {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => (i as i32 + delta).clamp(0, self.visible_jobs.len() as i32 - 1) as usize,
            None => 0,
        };
        self.table_state.select(Some(i));
        self.scrollbar_state = self.scrollbar_state.position(i);
        self.refresh_data();
    }

    fn draw_help(&self, f: &mut Frame) {
        let area = centered_rect(50, 40, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title("Help")
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::DarkGray));
        let text = "[Keys]\nq: Quit\nr: Refresh\nTab: Switch View\nj/k: Nav\n?: Toggle Help";
        f.render_widget(
            Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center),
            area,
        );
    }
}

fn centered_rect(px: u16, py: u16, r: Rect) -> Rect {
    let popup = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - py) / 2),
            Constraint::Percentage(py),
            Constraint::Percentage((100 - py) / 2),
        ])
        .split(r)[1];
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - px) / 2),
            Constraint::Percentage(px),
            Constraint::Percentage((100 - px) / 2),
        ])
        .split(popup)[1]
}
