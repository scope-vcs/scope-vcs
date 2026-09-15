-- Run only in the isolated restored database before starting its services with
-- a fresh, empty cache bucket. Durable application data is not removed.
BEGIN;
TRUNCATE scope_cache_deletion_queue, scope_cache_references, scope_cache_objects,
         scope_cache_orphan_uploads, scope_cache_uploads;
COMMIT;
