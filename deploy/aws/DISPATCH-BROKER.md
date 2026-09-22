# Validated runner dispatch

The Railway worker invokes one AWS Lambda function. It sends an attempt ID and
bootstrap token for start, or an attempt ID for stop. The broker asks the API to
authorize that operation from durable attempt state. The API supplies the job's
digest-pinned image and attempt deadline. The worker cannot supply AWS roles,
secret references, networking, resource limits, or task ARNs.

Custom workflow images remain supported. Every task has the existing runtime
entrypoint, 4 vCPU, 16 GiB memory, fixed network and execution role, and no task IAM
role. An optional private registry credential remains an exact configured secret
used only for pulls from `SCOPE_REGISTRY_CREDENTIALS_HOST`. Images from unrelated
registries do not receive those registry credentials.

## Ownership and credentials

- API and broker share `SCOPE_DISPATCH_BROKER_TOKEN`, a dedicated random credential
  of at least 32 characters. Never configure it on the worker. The API endpoint
  fails closed when the token is absent.
- Broker uses its Lambda execution role. It constructs definitions and secret
  names, passes only the configured ECS execution role, and owns privileged cleanup.
- Worker needs `AWS_REGION`, `SCOPE_DISPATCH_BROKER_FUNCTION_ARN`, and AWS credentials
  granting only `lambda:InvokeFunction` on that exact function. Cloud-run admission
  retains `SCOPE_CLOUD_RUNS_ENABLED`. `SCOPE_CLOUD_RUN_MAX_CONCURRENCY` defaults to 20;
  set it to 0 to pause new admission while cancellation and terminal cleanup
  continue in batches of ten.
- Remove direct ECS, Secrets Manager, and `iam:PassRole` grants from the worker
  identity. Remove old worker ECS settings, public API URL, registry ARN, and
  secret-name HMAC key once the cutover is verified.

The API checks current attempt identity, bootstrap hash, dispatching state, lease,
deadline, and cancellation without consuming the runtime bootstrap credential.
Stop permits canceled runs and terminal attempts, including older attempts still
requiring cleanup. The runtime continues exchanging its bootstrap credential once.

This boundary contains a stolen worker AWS credential. The worker currently also
has database write access to dispatch state. A full worker compromise with those
database privileges can tamper with the API's authoritative records. Protecting
against that stronger threat requires moving admission writes behind an owner
that the compromised worker cannot bypass.

## Durable failure handling

DynamoDB serializes each attempt with a 180-second lock. The Lambda has a
120-second hard timeout, so an expired lock cannot overlap a still-running old
invocation. Do not increase Lambda timeout above the lock duration. SDK timeouts
are bounded and automatic mutation retries are disabled.

The broker journals setup, the exact launch specification, and a launch marker
before calling ECS. It never calls `RunTask` again after an uncertain launch.
Repeated starts return the recorded task or discover it by `startedBy`. Unknown
outcomes remain owned by existing attempt lease recovery and cleanup.

Stop serializes with launch, records discovered task ARNs, and requires ECS
`STOPPED` confirmation before deleting definitions and the bootstrap secret. An
uncertain launch or registration waits at least five minutes before declaring
absence. Accepted stop requests are journaled so later polls only check task status.
The broker checks ECS at most once every ten seconds while cleanup is progressing;
`stopping` keeps durable cleanup ownership without an error warning. A stop
still pending after fifteen minutes is logged once. Real provider errors return
`ambiguous` and retain durable cleanup ownership. Definite ECS no-task outcomes
return `rejected` with a capacity, quota, or permanent reason after setup cleanup.
Replies use fixed messages for those reasons. The broker logs the AWS error code
and request ID when available, never the AWS error body.

Terminal journal records have no TTL and the table has deletion protection,
retention, and point-in-time recovery. Preserve these tombstones in recovery.
Deleting one can remove replay protection after ECS's 24-hour client-token window.
Drain attempts before changing the broker's cluster/network/execution configuration
or restoring an earlier journal snapshot.

## Build and verify

```bash
python3 -m unittest discover -s deploy/aws/dispatch-broker/tests -v
python3 deploy/aws/dispatch-broker/package.py /tmp/scope-dispatch-broker.zip
cargo test -p scope-domain dispatch_authorization
cargo test -p api dispatch_authorization
cargo test -p worker
```

The package command prints its SHA-256. The ZIP includes only the six runtime
modules. Python 3.13 Lambda supplies Boto3 and Botocore. Upload the reviewed ZIP to
a versioned deployment bucket and supply that exact object version to
`deploy/aws/dispatch-broker.yaml` through `CodeBucket`, `CodeKey`, and `CodeVersion`.

Deploy the security foundation first. The broker role requires the
`scope-runtime-boundary` managed policy. The broker stack also takes the existing
cluster ARN, subnets, security group, execution role ARN, runner log group, public
API HTTPS origin, and optional registry credential ARN plus its exact registry
host through `RegistryCredentialsHost`. Supply the authority token
through protected deployment input. `NoEcho` hides it in stack parameter listings;
principals allowed to inspect Lambda environment configuration remain privileged.

There is no Lambda function URL. IAM authorizes invocation. The broker's API call
uses verified HTTPS, rejects redirects, and bounds response size. Logs record only
the result status, invocation ID, and safe provider code and request ID when available,
never request bodies or AWS error payloads.

## Coordinated reply-format release

The worker strictly decodes the broker's rejection reason and `stopping` reply;
deploy the matching worker and broker as one coordinated release. First deploy the
new worker with `SCOPE_CLOUD_RUN_MAX_CONCURRENCY=0` against the existing broker.
Wait for active attempts and their cleanup claims to settle, while the worker
continues cancellation and terminal cleanup. Then deploy the new broker and restore
the intended positive concurrency value. Monitor broker `ambiguous` outcomes and
the journal before admitting new work. Do not run mixed reply formats during new
dispatch.

## Cutover and canary

1. Deploy API authorization with the dedicated token while leaving the current
   worker release running. Deploy the reviewed broker ZIP and stack.
2. Pause new cloud admission and drain existing direct-dispatch attempts through
   terminal task cleanup. Old bootstrap secret names contain an HMAC suffix; the
   broker deliberately has no legacy lookup path.
3. Update worker configuration and its invoke-only IAM policy, then deploy the
   broker-only worker. There is no direct-dispatch fallback.
4. Run a legitimate custom-image workflow. Verify task role is absent, execution
   role/network/limits are fixed, the runtime claims and completes, and the journal
   reaches `stopped` with bootstrap secret and definition cleanup.
5. Exercise cancellation, lost response/retry, setup rejection, and worker restart.
   Simulate the worker identity to confirm direct ECS/Secrets Manager/PassRole are
   denied. Invoke malformed image/role/secret/network payloads and verify rejection
   without new AWS resources.

Do not report cutover complete until these live checks pass. Reverting to direct
dispatch would require deliberately restoring the old administrative permissions
and would reopen the original security boundary.
