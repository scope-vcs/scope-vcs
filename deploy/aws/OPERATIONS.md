# Fargate cloud runner operations

CloudFormation owns the runner VPC, public subnets, route to the internet, security group, ECS cluster, log group, task execution role, private ECR runner base image repository, GitHub OIDC publisher role, dispatcher IAM user, exact private-registry secret grant, and optional budget. Do not create parallel resources in the AWS Console.

The cluster uses Fargate On-Demand. Each task gets a public IPv4 address because the runner must reach ECR, the Scope API, the cache, and source hosts. The security group has no inbound rules and permits outbound HTTPS only. There is no NAT gateway or idle compute cost. The runner base image lives in private ECR in the same region as Fargate. It contains only the runner runtime at `/scope/bin/scope-runner-runtime`, Git, and a shell. Workflows that need more tools build their own image from it.

The worker registers one `scope-runner-<attempt ID>` task definition per attempt because ECS cannot override either the container image or secret references in `RunTask`. The definition contains the digest-pinned image and a reference to a per-attempt Secrets Manager bootstrap credential. When `SCOPE_REGISTRY_CREDENTIALS_SECRET_ARN` is set, it also contains that exact ARN as ECS repository credentials. Neither credential value is placed in the ECS task override or returned by `DescribeTasks`. After ECS reports the task stopped, the worker deregisters the task definition and force-deletes the one-use bootstrap secret.

Each task is also keyed by its Scope attempt ID. The runtime has an absolute 24-hour watchdog, independent of the worker and heartbeat lease. The worker also hard-expires the database attempt and waits for ECS to report the old task as `STOPPED` before the job can dispatch another attempt. If `RunTask` succeeds but its response is lost, the worker polls for the task by attempt ID for five minutes, using bounded exponential backoff for ECS eventual consistency, before concluding no task was created. Cleanup claims run concurrently and are held for fifteen minutes so another worker cannot race that reconciliation.

## Prerequisites

- AWS CLI v2 authenticated to the production account
- GitHub CLI authenticated with repository administration access
- permission to manage CloudFormation, VPC, ECS, IAM, CloudWatch Logs, and AWS Budgets
- Railway CLI authenticated to the Scope production project when setting worker variables

Check the AWS identity before making a plan:

```bash
aws sts get-caller-identity
```

Never deploy this stack from the AWS root user. Use an administrative role with MFA for the initial stack and a CI deployment role for later changes.

The GitHub OIDC provider is account-global. Check for an existing provider before the first stack update:

```bash
aws iam list-open-id-connect-providers \
  --query "OpenIDConnectProviderList[?contains(Arn, 'token.actions.githubusercontent.com')].Arn" \
  --output text
```

If this prints an ARN, pass it as `EXISTING_GITHUB_OIDC_PROVIDER_ARN` when planning and applying. Otherwise the stack creates and owns the provider.

## Validate and preview

The script defaults to `us-east-1`, stack `scope-cloud-runner-production`, project `scope-vcs`, and environment `production`.

```bash
deploy/aws/apply-cloud-runner.sh validate
deploy/aws/apply-cloud-runner.sh plan
```

For an account that already has the GitHub provider:

```bash
EXISTING_GITHUB_OIDC_PROVIDER_ARN=arn:aws:iam::<account ID>:oidc-provider/token.actions.githubusercontent.com \
deploy/aws/apply-cloud-runner.sh plan
```

`plan` creates a CloudFormation change set, waits for AWS to prepare it, and prints every resource action. It does not execute the change set. For the first deployment, discard an unused preview by deleting its empty `REVIEW_IN_PROGRESS` stack:

```bash
aws cloudformation delete-stack \
  --region us-east-1 \
  --stack-name scope-cloud-runner-production
```

For later updates, delete only the unused change set with `aws cloudformation delete-change-set --change-set-name <change-set ARN>`.

Set `BUDGET_NOTIFICATION_EMAIL` to create a monthly project-tag budget. `MONTHLY_BUDGET_USD` defaults to 100.

```bash
BUDGET_NOTIFICATION_EMAIL=ops@example.com \
MONTHLY_BUDGET_USD=100 \
deploy/aws/apply-cloud-runner.sh plan
```

AWS Budgets can filter on the `Project=scope-vcs` tag only after the account activates that cost allocation tag. Check and activate it through the CLI:

```bash
aws ce list-cost-allocation-tags \
  --status Active \
  --tag-keys Project

aws ce update-cost-allocation-tags-status \
  --cost-allocation-tags-status TagKey=Project,Status=Active
```

Cost allocation tags can take up to 24 hours to appear. Omit `BUDGET_NOTIFICATION_EMAIL` until the tag is available if this is a new AWS account.

## Apply

`apply` validates the template, creates and prints a fresh change set, executes it, waits for the stack, and prints its outputs.

If an initial create reaches `ROLLBACK_COMPLETE`, the next `plan` or `apply` prints the latest stack events, deletes only that rolled-back empty stack, waits for deletion, and creates a fresh change set. Later update rollbacks remain intact for inspection and rollback.

```bash
BUDGET_NOTIFICATION_EMAIL=ops@example.com \
deploy/aws/apply-cloud-runner.sh apply
```

To execute the exact change set returned by `plan`, pass its ARN. This is required for the first deployment if the stack remains in `REVIEW_IN_PROGRESS` after a preview:

```bash
deploy/aws/apply-cloud-runner.sh apply <change-set ARN>
```

The script exits successfully when the stack already matches the template.

Apply this infrastructure before merging a workflow that publishes to ECR. Then configure the one non-secret GitHub Actions variable from the CloudFormation output. A stack that still publishes the checks image needs the staged update in [Move publishing to the runner base image](#move-publishing-to-the-runner-base-image) first.

```bash
publisher_role_arn="$(aws cloudformation describe-stacks \
  --region us-east-1 \
  --stack-name scope-cloud-runner-production \
  --query "Stacks[0].Outputs[?OutputKey=='RunnerBaseImagePublisherRoleArn'].OutputValue | [0]" \
  --output text)"

gh variable set SCOPE_RUNNER_IMAGE_AWS_ROLE_ARN \
  --repo scope-vcs/scope-vcs \
  --body "$publisher_role_arn"
```

GitHub receives temporary AWS credentials through OIDC. There is no AWS access key to create or store for image publishing. The role accepts only this repository's `main` branch and the `scope-runner-image.yml` reusable workflow, and it can write only the runner base image repository.

Configure the infrastructure role at the same time:

```bash
infrastructure_role_arn="$(aws cloudformation describe-stacks \
  --region us-east-1 \
  --stack-name scope-cloud-runner-production \
  --query "Stacks[0].Outputs[?OutputKey=='GitHubInfrastructureRoleArn'].OutputValue | [0]" \
  --output text)"

gh variable set SCOPE_AWS_INFRASTRUCTURE_ROLE_ARN \
  --repo scope-vcs/scope-vcs \
  --body "$infrastructure_role_arn"
```

Complete the [security bootstrap](SECURITY.md) before using the `AWS infrastructure` GitHub workflow. Configure the protected `AWS-infrastructure` environment and set `SCOPE_AWS_EXECUTION_ROLE_ARN` to the constrained CloudFormation execution role. Run `plan`, review its change-set ARN, then run `apply` with that exact ARN.

The GitHub role can manage change sets only for the two production runner and broker stacks and pass the fixed execution role. Its trust requires the immutable repository ID, `main`, the protected environment, and the exact reusable execution workflow. The execution role can update named production resources and bounded runtime roles; it cannot change its own permissions, GitHub trust, or security controls. Network replacement, budgets, and identity bootstrap use temporary non-root administration. Direct pushes to `main` remain independent of infrastructure approval.

## Configure broker dispatch

Follow [the broker deployment and cutover guide](DISPATCH-BROKER.md) before
releasing the broker-only worker. Deploy the API authorization endpoint and broker,
pause admission, and drain old attempts through cleanup before changing worker
permissions. The broker owns task definitions, bootstrap secrets, role passing,
networking, and cleanup. The worker invokes the exact broker Lambda ARN.

Set `AWS_REGION`, `SCOPE_DISPATCH_BROKER_FUNCTION_ARN`, and invoke-only AWS
credentials on the worker. The API and broker share the dedicated
`SCOPE_DISPATCH_BROKER_TOKEN`; the worker must never receive it. Broker infrastructure
receives the cluster, subnet, security group, execution role, and log group settings.
Remove the worker's old ECS settings and `SCOPE_ECS_SECRET_NAME_KEY` after cutover.

CloudFormation creates the dispatcher IAM user without creating a permanent access
key. Create and transfer any required key through a protected shell, then verify
that it can invoke only the broker and cannot call ECS, Secrets Manager, or
`iam:PassRole` directly. Do not save key material in command examples or logs.

The API independently validates attempt state and bootstrap identity without
consuming the runtime credential. The broker selects the durable workflow image
and fixes all privileged AWS inputs. Tasks have no task IAM role. Arbitrary
registered task definitions and secret-name obscurity are not security boundaries;
ECS deregistration can return secret references. That wildcard-only permission now
belongs to the trusted broker.

## Configure private registry credentials

Public images need no registry credential. For a private registry, create an
AWS Secrets Manager secret in the runner region with this JSON shape:

```json
{"username":"registry-user","password":"registry-password-or-token"}
```

Use the AWS-managed `aws/secretsmanager` key. A customer-managed key additionally
requires an exact `kms:Decrypt` grant, which this stack does not provide. Configure
the exact ARN in the runner stack's `RegistryCredentialsSecretArn` parameter so
the execution role can pull from that registry. Supply the same ARN and the exact
registry hostname as `RegistryCredentialsHost` to the broker stack.

The broker supplies `repositoryCredentials` only when the image's registry matches
that hostname. Other public registries remain credential-free. The registry secret
is never injected into the container environment or configured on the worker.
Rotate its value under the same ARN, verify a private-image canary, and revoke the
old registry token. Never place the secret JSON or token in repository variables.

## Observe a real run

Tail container output:

```bash
aws logs tail /scope-vcs/production/cloud-runner \
  --region us-east-1 \
  --follow
```

Inspect active and stopped tasks:

```bash
cluster_arn="$(aws cloudformation describe-stacks \
  --region us-east-1 \
  --stack-name scope-cloud-runner-production \
  --query "Stacks[0].Outputs[?OutputKey=='RunnerClusterArn'].OutputValue | [0]" \
  --output text)"

aws ecs list-tasks --region us-east-1 --cluster "$cluster_arn"
aws ecs describe-tasks \
  --region us-east-1 \
  --cluster "$cluster_arn" \
  --tasks <task ARN>
```

Check the task's image digest, exit code, stopped reason, and timestamps. Confirm that aborting a Scope run stops its exact ECS task.

For the public mode proof, run a digest-pinned image from a registry without configured credentials and confirm that the task definition has no `repositoryCredentials`. For the private mode proof, apply the exact-secret execution-role grant and configure the broker ARN and registry host, then run a digest-pinned private image. Confirm that the task reaches `RUNNING`, claims its Scope attempt, and references the exact configured ARN. Do not print or fetch the secret value during either proof.

Use IAM simulation after the stack update. The dispatcher must be denied `secretsmanager:GetSecretValue` for both the registry secret and a sample attempt secret. The task execution role must be allowed for the exact registry ARN and the attempt prefix, and denied for an unrelated secret.

## Move publishing to the runner base image

A stack that still holds the `scope-vcs/production/checks` repository needs this update once. It keeps the publisher role `scope-cloud-runner-production-github-checks-publisher` and its name, and changes only its trust conditions and repository grant. The protected `scope-infrastructure-execution` role may not change GitHub roles, so a routine apply of this change rolls back. Run every step below with the temporary administration identity from [SECURITY.md](SECURITY.md), from the change's branch, before merging it and before any release that includes it. The first such release selects the new `runner-image` component and publishes at once.

1. Let the execution role manage the new repository. Update the security stack from `security-deployment-role.yaml`, review the change set, then execute it:

   ```bash
   aws cloudformation create-change-set      --region us-east-1      --stack-name scope-security-deployment      --change-set-name runner-base-repository      --template-body file://deploy/aws/security-deployment-role.yaml      --parameters ParameterKey=RegistryCredentialsSecretArn,UsePreviousValue=true      --capabilities CAPABILITY_NAMED_IAM
   aws cloudformation describe-change-set --region us-east-1      --stack-name scope-security-deployment --change-set-name runner-base-repository
   aws cloudformation execute-change-set --region us-east-1      --stack-name scope-security-deployment --change-set-name runner-base-repository
   aws cloudformation wait stack-update-complete --region us-east-1      --stack-name scope-security-deployment
   ```

2. Create a temporary CloudFormation service role that may change the publisher role:

   ```bash
   aws iam create-role      --role-name scope-runner-publisher-migration      --assume-role-policy-document '{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"Service":"cloudformation.amazonaws.com"},"Action":"sts:AssumeRole"}]}'
   aws iam attach-role-policy      --role-name scope-runner-publisher-migration      --policy-arn arn:aws:iam::aws:policy/AdministratorAccess
   ```

3. Plan the runner stack with that role:

   ```bash
   SCOPE_AWS_EXECUTION_ROLE_ARN="arn:aws:iam::$(aws sts get-caller-identity --query Account --output text):role/scope-runner-publisher-migration"    deploy/aws/apply-cloud-runner.sh plan
   ```

   Pass the same `BUDGET_NOTIFICATION_EMAIL`, `MONTHLY_BUDGET_USD` and `SCOPE_REGISTRY_CREDENTIALS_SECRET_ARN` as routine applies. The change set must add `RunnerBaseImageRepository`, remove `ChecksImageRepository`, and modify `ChecksImagePublisherRole` and `RunnerTaskExecutionRole` with `Replacement` `False`. Stop if it shows anything else, in particular a change to `GitHubOidcProvider` or `GitHubInfrastructureRole`.

4. Apply that exact change set with the same environment, then confirm the role ARN output:

   ```bash
   deploy/aws/apply-cloud-runner.sh apply <change-set ARN>
   ```

5. Delete the temporary role:

   ```bash
   aws iam detach-role-policy      --role-name scope-runner-publisher-migration      --policy-arn arn:aws:iam::aws:policy/AdministratorAccess
   aws iam delete-role --role-name scope-runner-publisher-migration
   ```

   CloudFormation keeps the last service role on the stack, so it now names a role that no longer exists. Every routine plan passes `scope-infrastructure-execution`, and the next applied change associates it again. Check `aws cloudformation describe-stacks --stack-name scope-cloud-runner-production --query 'Stacks[0].RoleARN'` after that apply.

6. Set `SCOPE_RUNNER_IMAGE_AWS_ROLE_ARN` from the `RunnerBaseImagePublisherRoleArn` output, as shown above. The ARN is the same role the checks image used.

7. Merge, then release. After the release publishes the runner base image, delete the retained `scope-vcs/production/checks` repository, the `SCOPE_CHECKS_IMAGE_AWS_ROLE_ARN` variable, and the `checks` package on GHCR:

   ```bash
   aws ecr delete-repository --region us-east-1      --repository-name scope-vcs/production/checks --force
   gh variable delete SCOPE_CHECKS_IMAGE_AWS_ROLE_ARN --repo scope-vcs/scope-vcs
   ```

   Delete the GHCR package from the organization's package settings.

Between step 4 and the merge, the publisher role accepts only `scope-runner-image.yml` on `main`, so a release from the old `main` that selects the checks image fails to publish it. Do not release in that window.

## Publish the runner base image

`runner-runtime/Dockerfile` builds the runner base image. When a change selects the `runner-image` component, pull requests and Scope request branches build the image, run `dev/checks/runner-runtime-container` against it, and scan it. A release that selects the component repeats those checks on `main`, then pushes the image to `scope-vcs/production/runner-base` under a tag unique to the run. The `runner-image-<commit>` artifact and the run summary record the digest. Pin workflows to that digest, not the tag. The repository retains tagged images; its lifecycle policy deletes only untagged artifacts older than fourteen days.

## Disable and roll back

Pause new admission with `SCOPE_CLOUD_RUN_MAX_CONCURRENCY=0` before changing infrastructure; cancellation and cleanup keep running. Stop any remaining task by ARN:

```bash
aws ecs stop-task \
  --region us-east-1 \
  --cluster "$cluster_arn" \
  --task <task ARN> \
  --reason "Scope Fargate rollback"
```

Queued jobs remain in Scope and can resume after the worker is fixed. Do not delete the stack as a first response because stack deletion also removes the log group and its diagnostic logs.

## Rotate or revoke the dispatcher key

Create a second key, update Railway, verify one run, then remove the old key:

```bash
aws iam list-access-keys --user-name "$dispatcher_user"
aws iam create-access-key --user-name "$dispatcher_user"
aws iam delete-access-key \
  --user-name "$dispatcher_user" \
  --access-key-id <old access key ID>
```

IAM allows at most two keys per user. Revoke both keys immediately if either secret leaves the intended secret store.
