# Resource & topology detection

UnifiedLab tries to behave sensibly across:
- laptops (Local)
- Slurm clusters
- PBS clusters
- LSF clusters

It detects the environment by reading standard environment variables and system state.

---

## Cluster detection

The cluster type is inferred from env vars:

- Slurm: `SLURM_JOB_ID`
- PBS: `PBS_JOBID`
- LSF: `LSB_JOBID`
- otherwise: Local

---

## Rank detection

UnifiedLab tries Slurm first, then common MPI variables:

- Slurm: `SLURM_PROCID`, `SLURM_NTASKS`
- OpenMPI: `OMPI_COMM_WORLD_RANK`, `OMPI_COMM_WORLD_SIZE`
- PMI: `PMI_RANK`, `PMI_SIZE`
- MVAPICH: `MV2_COMM_WORLD_RANK`, `MV2_COMM_WORLD_SIZE`

If nothing is set, it assumes `(rank=0, world_size=1)`.

---

## Wrapping sub-commands (job steps)

When UnifiedLab launches an external binary inside an allocation, it wraps the command so it consumes only the resources it asked for:

- On **Slurm**, it uses `srun --exclusive --exact --nodes=… --ntasks=…`
- On **PBS/LSF**, it uses `mpirun -np …` when needed
- On **Local**, it runs directly

This behaviour is crucial if you want multiple concurrent jobs inside one allocation without everyone fighting over the whole node.

---

## Practical tips

- If scheduling looks odd, temporarily use `--limit-cores` to shrink the world and make behaviour obvious.
- Use node `--tags` as a way to annotate special machines (GPUs/high-mem) even before you implement fancy scheduling rules.
