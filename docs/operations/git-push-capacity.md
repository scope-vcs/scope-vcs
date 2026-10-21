# Git push capacity

Push throughput scales by adding API replicas. Each replica admits a fixed number of concurrent pushes, and every admitted push can hold Postgres connections. Size the replica count and the per-replica settings together so the worst case stays inside the database's connection limit.

## API settings

| Variable | Default | Meaning |
| --- | --- | --- |
| `SCOPE_GIT_RECEIVE_PACK_CONCURRENCY` | 4 | Concurrent pushes per replica. |
| `SCOPE_GIT_SEGMENT_INGEST_CONCURRENCY` | 4 | Concurrent pack uploads to object storage per replica. Keep it equal to receive-pack concurrency when pushes are small. |
| `SCOPE_DATABASE_MAX_CONNECTIONS` | 10 | Size of the replica's main Postgres pool. |
| `SCOPE_GIT_ADMISSION_WAIT_MS` | 2000 | How long a push or fetch waits for a free slot before the API returns 429. |
| `SCOPE_GIT_UPLOAD_PACK_CONCURRENCY` | 8 | Concurrent fetches per replica. |
| `SCOPE_GIT_COMMAND_TIMEOUT_SECS` | 30 | Deadline for each Git subprocess. |

## Worker settings

| Variable | Default | Meaning |
| --- | --- | --- |
| `SCOPE_GIT_COMPACTION_CONCURRENCY` | 1 | Compaction jobs the run worker processes at once. Its Postgres pool grows by the same amount. |

Compaction merges two segments per job. Under sustained pushes to the same repositories, segment counts grow faster than one compactor clears them; raise this setting when due rows in `scope_git_compaction_jobs` keep growing.

## Router settings

| Variable | Default | Meaning |
| --- | --- | --- |
| `SCOPE_REPO_ROUTER_READ_REPLICAS` | 1 | API replicas a repository's fetches rotate across. |
| `SCOPE_REPO_ROUTER_WRITE_REPLICAS` | 1 | API replicas a repository's pushes rotate across. |

The router ranks API replicas per repository. With the default of 1, every push for a repository goes to its top-ranked replica, so one busy repository is limited to one replica's CPU. Raising `SCOPE_REPO_ROUTER_WRITE_REPLICAS` spreads its pushes across that many replicas. Request revisions, main pushes, and request merges prepare their packs without a repository lock and commit by compare-and-swap on the repository head in Postgres, so the replica a push lands on does not affect correctness. A main push whose base moved during preparation is rejected so the client fetches and pushes again; a request merge prepares again on the new main, up to five attempts. Replicas share request branch objects through the request object pool: before preparing a push, a replica installs the packs other replicas registered since its last sync. A replica that has not synced yet restores a request from its snapshot instead.

## Connection budget

Worst-case Postgres connections:

```
api_replicas × (SCOPE_DATABASE_MAX_CONNECTIONS + 1)
  + worker_replicas × (10 + SCOPE_GIT_COMPACTION_CONCURRENCY)
  + media services, maintenance, and superuser_reserved_connections
```

Keep that total below Postgres `max_connections`. The bound applies when every pool is full at once.

On Railway, the Postgres container allows about 1,000 processes. Each connection is a process, so `max_connections` above roughly 900 fails with fork errors under load rather than with a clean connection refusal.

## Postgres settings for high push rates

These settings were measured on the pr-668 load environment. They are not applied automatically.

| Setting | Value | Effect |
| --- | --- | --- |
| `max_connections` | Sized from the budget above | Admits every replica's pools. |
| `commit_delay` | `1000` (microseconds) | Groups concurrent commits into one WAL flush. |
| `wal_init_zero` | `off` | Skips zero-filling new WAL segments on copy-on-write volumes. |
| `min_wal_size` | `8GB` | Recycles WAL segments instead of creating new ones. |

Keep `synchronous_commit` on. Turning it off removed the WAL flush stalls but did not raise the ceiling past about 920 pushes per second, and it gives up durability of acknowledged pushes.

## Scaling procedure

1. Pick per-replica settings. The load environment used 128 receive-pack and segment-ingest slots with a 32-connection main pool.
2. Compute the connection budget for the target replica count and raise `max_connections` first if needed. Changing it restarts Postgres.
3. Set the API and worker variables, then change the API replica count.
4. Watch 429 rates for push admission, Postgres connection counts, and compaction candidates.

## Measured ceiling

On six API replicas against one Railway Postgres volume, 4 KiB pushes reached about 900 per second and 1 MiB pushes about 717 per second, with durable commits. The limit was the Postgres volume: it caps writes at 70 MB/s and periodically stalls WAL flushes for one to two seconds. More replicas do not raise that ceiling; a faster database volume does.
