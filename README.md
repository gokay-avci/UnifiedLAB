# UnifiedLab: HPC Active Learning Orchestrator

UnifiedLab is a high-performance orchestration engine designed for scientific workflows. It combines the reliability of Rust with the flexibility of Python-based active learning agents, all controlled via a visual Draw.io interface.

## 🚀 Features

-   **Visual Workflow Design**: Design your experiments as flowcharts in Draw.io.
-   **Hybrid Orchestration**: Seamlessly manage HPC jobs (VASP, GULP, Janus) and AI Agents.
-   **Active Learning**: Agents can dynamically spawn new simulations based on results.
-   **Resilience**: Built-in checkpointing and crash recovery.
-   **TUI Dashboard**: Real-time monitoring of your cluster and job queues.

## 📦 Installation

UnifiedLab requires **Rust** (for the core) and **uv** (for Python environment management).

### Prerequisites

1.  **Install Rust**: [rustup.rs](https://rustup.rs/)
2.  **Install Just**: [just.systems](https://github.com/casey/just) (Command runner)
3.  **Install uv**: [astral.sh/uv](https://docs.astral.sh/uv/) (Fast Python package manager)

### Quick Setup

Clone the repository and run:

```bash
just setup
```

This will:
-   Check your Rust installation.
-   Set up a Python virtual environment with `uv`.
-   Install necessary Python drivers (`numpy`, etc.).

## 🏁 Getting Started

### 1. Start the Service

Start the UnifiedLab Node Service (Guardian + Coordinator). By default, it runs in "Local Mode" for testing.

```bash
just start
```

### 2. Design a Workflow

1.  Open [Draw.io](https://app.diagrams.net/) or the desktop app.
2.  Create a flowchart:
    -   **Rectangle**: Represents a Compute Job.
    -   **Arrows**: Represent Dependencies (A -> B means B runs after A).
    -   **Label**: The name of the job. Include "Janus" in the name to use the GPU engine.
3.  **Important**: Save the file as **Uncompressed XML** (`.drawio`).
    -   *File > Export as > XML > Uncheck "Compressed"*

### 3. Deploy the Workflow

Submit your `.drawio` file to the running service:

```bash
just deploy file="my_experiment.drawio"
```

(Or use the included example: `just deploy`)

### 4. Monitor Progress

Launch the Terminal User Interface (TUI) to watch your jobs execute in real-time:

```bash
just tui
```

## 🧠 Workflow Engines

The system infers the "Engine" (Solver) from the job name/label:

-   **Janus**: GPU-accelerated ML Potential (Label contains "janus").
-   **Agent**: Python script for decision making (Default).
-   **GULP / VASP**: (Mock implementations available for testing).

## 🛠️ Development

-   **Build**: `just build`
-   **Test**: `just test`
-   **Format**: `just fmt`
-   **Clean**: `just clean`

## 🤝 Contributing

See `TODO.md` for a list of planned improvements.
