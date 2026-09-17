# AWS security operations

Apply the account controls in `us-east-1`. CloudTrail records management activity in all regions. IAM events and console root sign-ins reach the regional EventBridge rules in `us-east-1`. Root API activity and GuardDuty findings in other regions require matching rules in those regions. The current stack does not claim account-wide alert delivery.

## Bootstrap and routine deployment

1. Establish a non-root temporary administration identity. Root has MFA and no permanent access keys, but a root CLI login remains root. Supply its successor's exact user or role ARN as `AuditPrincipalArn`. An empty parameter omits the audit role and leaves this migration unfinished.
2. Validate and plan `security-controls.yaml` using `apply-security-controls.sh validate` and `plan`. Set `SECURITY_ALERT_EMAIL` explicitly on first creation. On updates, unset parameter environment variables preserve deployed values; setting `SECURITY_AUDIT_PRINCIPAL_ARN` to an empty string explicitly removes the audit role. Review the generated change set and apply its exact ARN. Confirm the SNS email subscription. Check CloudTrail `get-trail-status` for delivery errors, then confirm a management event arrives in the versioned bucket.
3. Bootstrap `security-deployment-role.yaml` as stack `scope-security-deployment`. Set `RegistryCredentialsSecretArn` to the exact existing private registry secret when used. Its boundary permits only the runner and broker's runtime operations.
4. Update the existing runner stack through the temporary administration identity to install the boundary and replace the GitHub role's administrator policy. Bootstrap changes to GitHub trust, IAM identities, networks and budgets require that privileged path. Routine CI cannot modify them.
5. Create GitHub environment `AWS-infrastructure`, restrict deployment branches to `main`, and require an owner review for infrastructure runs. Set `SCOPE_AWS_EXECUTION_ROLE_ARN` to the output from the deployment security stack. Direct pushes to main continue normally; only the manually dispatched infrastructure job uses the protected environment.
6. Remove old unexecuted change sets created with elevated authority. Associate both production stacks with the fixed `scope-infrastructure-execution` role. CloudFormation retains its service role across operations, so review that stored ARN before allowing CI access.
7. Dispatch `AWS infrastructure` with `validate`, then `plan`. Verify the reusable job assumes the expected role. Review the plan and apply its exact ARN. Check that another branch, workflow or environment cannot assume the role.

The CI principal can plan and execute changes only on `scope-cloud-runner-production` and `scope-dispatch-broker-production`. Its service role can change the named production ECS, ECR, logging, broker Lambda and journal resources. IAM writes are confined to two runtime roles. New roles require the fixed permission boundary; deleting the boundary is denied. The service role cannot mutate itself, GitHub roles, OIDC trust providers or security-owned roles/policies. It cannot read secret values or assume another role.

Existing runner networking may be changed only where the EC2 resource carries the runner stack's CloudFormation system tag. Network replacement and budget changes require the privileged bootstrap path. The broker code bucket is `scope-dispatch-broker-artifacts-ACCOUNT-REGION`; retain versioning and supply an exact code object version. The CI role has no upload permission.

## Human enrollment and routine audit login

`human-access.yaml` creates the console-only `scope-operator` user. The existing security-controls stack remains the sole owner of the metadata-only audit role. Deploy the human stack, then set `AuditPrincipalArn` on the security-controls stack to its `UserArn` output. The audit trust requires that exact principal and MFA. No administrator role or permanent API key is created.

The account owner must enable console access in IAM and personally choose and store the password. No login profile or password is stored in the template or passed through the agent. Open `UserSecurityCredentialsUrl` from the human stack outputs. While signed in as the account owner, assign two independent passkeys/security keys to the new user using the browser. AWS also supports the user enrolling their own passkey from My security credentials. The user policy allows only its own MFA enrollment actions before MFA authentication. [AWS passkey enrollment](https://docs.aws.amazon.com/IAM/latest/UserGuide/id_credentials_mfa_enable_fido.html)

The policy denies password changes before MFA. Do not enable “require password reset at next sign-in” when creating console access. The human chooses the initial password, enrolls MFA, then signs in again with MFA before changing that password if needed. Do not grant access-key management to make the Security credentials page fully available. The unrelated sections intentionally remain denied. [AWS MFA and initial-password guidance](https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_policies_examples_aws_my-sec-creds-self-manage.html)

For daily audit access:

1. Use `SignInUrl` to sign in as `scope-operator` with MFA.
2. Use `AuditSwitchRoleUrl` to switch into `scope-security-audit`.
3. Run `aws login --profile scope-audit --region us-east-1` and select that audit role console session. Use `--remote` when the CLI runs on another machine. This requires AWS CLI 2.32.0 or newer. [AWS CLI console-session login](https://docs.aws.amazon.com/en_en/cli/latest/userguide/cli-configure-sign-in.html)
4. Check `aws --profile scope-audit sts get-caller-identity`. It must identify `assumed-role/scope-security-audit`, never root. Verify metadata reads succeed while secret reads, application-content reads, mutations and further role assumption are denied.

The role can create CLI login tokens only for the account's `us-east-1` local and remote OAuth clients. The user's unenrolled session cannot create CLI tokens or assume the role. Test that MFA gate before completing enrollment. After the positive and negative canaries succeed, log out the cached root CLI profile.

The human user cannot deactivate MFA. Keep a second enrolled device and root's protected recovery material. Device replacement or loss recovery uses the account owner's console. This replaces routine root audits; privileged administration remains a separate operation.

## Retention and paid monitoring

The private audit bucket uses SSE-S3 encryption, versioning, TLS enforcement, a 365-day current and noncurrent version lifecycle, and retained bucket/trail resources on stack removal. Log file validation detects tampering; it does not make the bucket immutable. Keep privileged access to this stack separate from deployment CI.

`EnableGuardDuty=false` is deliberate. Its detector would cover the deployment region only. Before enabling it, inspect whether a detector already exists, estimate CloudTrail event volume, VPC flow/DNS analysis and optional protection-plan usage, and review current regional GuardDuty prices. No fixed monthly estimate is defensible without that usage. The findings alert rule is installed even when detector creation is disabled. Security Hub is not required.

## Validation

```sh
python3 deploy/aws/security-controls-contract.test.py
node --test deploy/aws/cloud-runner-contract.test.mjs
bash -n deploy/aws/apply-cloud-runner.sh deploy/aws/apply-security-controls.sh
```

The Python contracts require PyYAML. Also run AWS `cloudformation validate-template` on both security templates and review IAM Access Analyzer policy validation before applying. Policy contracts check intended boundaries, not AWS authorization behavior. Complete live positive and negative OIDC, role passing and secret-access checks after the staged rollout.

## Production backup monitoring

`backup-monitor.yaml` creates the exact-topic CloudWatch alarm and a GitHub OIDC role that can only publish metrics in `Scope/Security/Backup`. Deploy it after the account controls, passing `AlertTopicArn` and the existing GitHub OIDC provider ARN. Update the account topic policy to include the backup alarm's service grant. Set repository variable `SCOPE_BACKUP_MONITOR_ROLE_ARN` from the output.

The `Backup monitor` workflow is scheduled every fifteen minutes because GitHub drops most scheduled runs and the alarm pages after three hours without a heartbeat. It uses the existing production-project `RAILWAY_TOKEN`. Railway has not provided a verified read-only capability for this token; its project scope is the narrowest existing credential. The workflow forwards only this secret to its reusable job. It checks that DAILY and WEEKLY volume schedules exist and that the production volume has an unexpired completed snapshot no older than 30 hours. Railway's list response has no separate completion status, so the check uses the persisted snapshots returned by that API.

Failed queries, missing schedules and stale backups publish a zero health metric and fail the workflow. Healthy checks publish one. Three consecutive hours containing a failure or no metric trigger the alarm, so a disabled schedule or broken AWS authentication is observable. A single transient query failure does not page. This checks backup availability; it does not prove restorability or cover blob/key recovery. The monitored volume ID is fixed in the script and alarm and must change together if production storage is replaced.

After deployment and merge, dispatch the monitor once and inspect its summary and CloudWatch metric. Test alert delivery by setting the alarm to ALARM with an explicit test reason, confirm receipt, then let the next health evaluation restore its state. Email subscription confirmation is required before either real or test alerts arrive.
