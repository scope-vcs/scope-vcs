-- Run inside the same exported read-only snapshot as pg_dump. Content references
-- are the existing ownership index; GitBlob content lives in the segment ledger.
SELECT 'content' AS kind, 'objects' AS bucket, object_key::jsonb AS content_ref,
       NULL::text AS key, NULL::text AS sha256, NULL::text AS repo_id,
       NULL::text AS segment_id, NULL::bigint AS plaintext_bytes,
       NULL::text AS manifest_id, NULL::integer AS chunk_index,
       NULL::text AS manifest_sha256, NULL::bigint AS manifest_bytes
FROM scope_object_references
UNION ALL
SELECT 'segment', 'objects', NULL, object_key, sha256, repo_id, segment_id,
       plaintext_bytes, NULL, NULL, NULL, NULL
FROM scope_git_segment_uploads WHERE state IN ('ready', 'published', 'retained')
UNION ALL
SELECT 'media', 'media', NULL, chunk.object_key, chunk.sha256, NULL, NULL,
       chunk.plaintext_size_bytes, manifest.id, chunk.chunk_index,
       manifest.sha256, manifest.size_bytes
FROM scope_request_media_manifest_chunks chunk
JOIN scope_request_media_manifests manifest ON manifest.id = chunk.manifest_id
UNION ALL
SELECT 'media', 'media', NULL, object_key, sha256, NULL, NULL,
       plaintext_size_bytes, NULL, NULL, NULL, NULL
FROM scope_request_media_upload_parts WHERE state = 'Stored'
UNION ALL
SELECT 'cache', 'cache', NULL, object_key, checksum_sha256, NULL, NULL,
       size_bytes, NULL, NULL, NULL, NULL
FROM scope_cache_objects;
