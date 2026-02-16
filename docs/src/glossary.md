# Glossary

**Allocation**  
A reserved set of nodes/cores on a cluster (e.g. a Slurm job allocation).

**Blueprint**  
Your Draw.io workflow diagram. UnifiedLab imports this into a job graph.

**Checkpoint**  
SQLite DB (`checkpoint.db`) storing “current truth” about workers and jobs.

**Coordinator**  
Rank 0 process that schedules work and maintains global state.

**Event log**  
Append-only binary+JSON log with magic header + CRC for robustness.

**Inbox**  
A folder under `root/` where submissions and worker messages land as logs.

**Job**  
A unit of work derived from a blueprint node.

**Worker**  
A process that requests work, executes it, and reports completion.

**Work grant**  
Coordinator’s response to a worker request: “run this job next”.
