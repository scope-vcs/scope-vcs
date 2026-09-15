# Recovery rollout and first drill

Run infrastructure operations with the separately authorized account administration
session. The ordinary infrastructure deployment role deliberately cannot manage
this protected recovery stack. No new AWS access keys are needed.

## Deploy and activate

1. Merge the reviewed recovery code to main. Update the security foundation stack
   with `AllowRecoverySetHealthAlarm` before expecting SNS delivery.
2. Preserve a newly generated age identity in the owner's offline recovery storage;
   record only its public recipient in GitHub. Do not print the private identity or
   send it through a GitHub secret.
3. Deploy the protected stack, substituting reviewed, nonsecret account values:

```bash
aws cloudformation deploy --region us-east-1 \
  --stack-name scope-production-recovery \
  --template-file deploy/aws/recovery/storage.yaml \
  --capabilities CAPABILITY_NAMED_IAM \
  --parameter-overrides \
    GitHubOidcProviderArn="arn:aws:iam::$ACCOUNT_ID:oidc-provider/token.actions.githubusercontent.com" \
    ReaderPrincipalArn="arn:aws:iam::$ACCOUNT_ID:user/scope-operator" \
    AlertTopicArn="$SECURITY_ALERT_TOPIC_ARN"
```

4. Read stack outputs. Set `SCOPE_RECOVERY_WRITER_ROLE_ARN`,
   `SCOPE_RECOVERY_BUCKET`, and `SCOPE_RECOVERY_AGE_RECIPIENT` repository variables.
   Reuse the existing Railway token and the production environment private SSH key
   secret. Keep the key in that environment; the reusable job selects production.
   Ensure writer trust uses the production environment subject together with the
   existing independent main-branch and exact reusable-workflow conditions. Check the
   deployment manifest has the production maintenance and media service IDs.
5. Dispatch and watch the initial workflow:

```bash
gh workflow run recovery.yml --ref main
gh run list --workflow recovery.yml --limit 1
gh run watch "$RECOVERY_RUN_ID" --exit-status
```

A successful job must publish a receipt naming a non-null archive version and
completion-marker version. Record those exact identifiers and the archive SHA-256
as private operational evidence. Verify `RecoverySetComplete=1` for this bucket;
separately inspect the alarm and SNS policy. Daily capture is at 07:17 UTC.

## Read with a separate MFA session

Sign into the existing `scope-operator` console user with MFA and switch to
`scope-recovery-reader` using the stack's switch-role URL. Use `aws login` to
create a temporary CLI login for that selected role. Verify `aws sts
get-caller-identity` shows `assumed-role/scope-recovery-reader/` before downloading.
The role permits AWS CLI OAuth login only for the same exact public-client ARNs
as the audit role. It cannot write objects. The metadata audit role remains
unable to read backup contents. Never create permanent access keys for this drill.

```bash
umask 077
aws s3api get-object --bucket "$RECOVERY_BUCKET" \
  --key "$COMPLETION_KEY" --version-id "$COMPLETION_VERSION" complete.json
aws s3api get-object --bucket "$RECOVERY_BUCKET" \
  --key "$ARCHIVE_KEY" --version-id "$ARCHIVE_VERSION" recovery.tar.age
python deploy/aws/recovery/restore.py recovery.tar.age \
  --identity /secure/offline-recovery-identity \
  --expected-sha256 "$ARCHIVE_SHA256" \
  --destination /secure/isolated-scope-recovery
```

Use the archive key, version, and checksum from the retrieved completion marker;
compare the marker version with the independent workflow receipt. The restore
verifies every required object using the escrowed keys and the same-snapshot
reference list. Restore its database into an empty, network-none PostgreSQL18
container with no published ports, run the reviewed maintenance schema verifier,
and apply `rebuild-cache.sql` there because the daily capture excludes cache.
Record only aggregate counts and hashes in drill output. Remove the isolated
plaintext files and container after preserving the encrypted archive and proof.

Do not claim an existing standalone database dump proves object or key recovery.
The first successful downloaded-archive drill establishes that proof; a full
application canary additionally needs new isolated buckets and app credentials as
described in README.md. Keep those claims separate in rollout evidence.
