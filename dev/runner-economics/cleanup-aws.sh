#!/usr/bin/env bash
set -euo pipefail
output=${1:?output directory required}
mkdir -p "$output"
stack=scope-cloud-runner-staging-mi-experiment
cluster=scope-vcs-staging-runner
dispatcher=scope-cloud-runner-staging-mi-experiment-railway-dispatcher
role=scope-staging-experiment-github-94d56973
admin_policy=arn:aws:iam::aws:policy/AdministratorAccess

test "$(aws sts get-caller-identity --query Account --output text)" = 957143340948
aws cloudformation describe-stacks --stack-name "$stack" > "$output/stack-before.json"
aws cloudformation list-stack-resources --stack-name "$stack" > "$output/resources-before.json"
test "$(aws ecs list-tasks --cluster "$cluster" --desired-status RUNNING --query 'length(taskArns)' --output text)" = 0
vpc=$(jq -er '.StackResourceSummaries[] | select(.ResourceType == "AWS::EC2::VPC") | .PhysicalResourceId' "$output/resources-before.json")
nat=$(jq -er '.StackResourceSummaries[] | select(.ResourceType == "AWS::EC2::NatGateway") | .PhysicalResourceId' "$output/resources-before.json")
aws ec2 describe-instances --filters "Name=vpc-id,Values=$vpc" > "$output/hosts-before.json"
ended=$(date -u +%FT%TZ)
for metric in BytesInFromSource BytesInFromDestination BytesOutToSource BytesOutToDestination; do
  aws cloudwatch get-metric-statistics --namespace AWS/NATGateway --metric-name "$metric" \
    --dimensions "Name=NatGatewayId,Value=$nat" --start-time 2026-09-07T15:30:00Z --end-time "$ended" \
    --period 60 --statistics Sum > "$output/nat-$metric.json"
done
aws ecr describe-images --repository-name scope-vcs/production/checks \
  --image-ids imageDigest=sha256:a3ed25023a60d000a8c93264518e9cdfd656f6832a6940a2eda7991db2ebbc9f \
  > "$output/checks-image.json"
aws pricing get-products --service-code AmazonECS --filters \
  Type=TERM_MATCH,Field=instanceType,Value=m8a.xlarge \
  Type=TERM_MATCH,Field=regionCode,Value=us-east-1 > "$output/managed-price.json"
aws logs filter-log-events --log-group-name /scope-vcs/staging/cloud-runner > "$output/runner-logs.json"

# Runtime-created resources are outside the CloudFormation resource graph.
mapfile -t definitions < <(aws ecs list-task-definitions --family-prefix scope-staging-runner- --status ACTIVE --output json | jq -r '.taskDefinitionArns[]')
for definition in "${definitions[@]}"; do
  aws ecs deregister-task-definition --task-definition "$definition" > /dev/null
done
mapfile -t definitions < <(aws ecs list-task-definitions --family-prefix scope-staging-runner- --status INACTIVE --output json | jq -r '.taskDefinitionArns[]')
for definition in "${definitions[@]}"; do
  aws ecs delete-task-definitions --task-definitions "$definition" > /dev/null
done
mapfile -t secrets < <(aws secretsmanager list-secrets --output json | jq -r '.SecretList[] | select(.Name | startswith("scope-vcs/scope-vcs-staging-runner/attempts/")) | .ARN')
for secret in "${secrets[@]}"; do
  aws secretsmanager delete-secret --secret-id "$secret" --force-delete-without-recovery > /dev/null
done
mapfile -t keys < <(aws iam list-access-keys --user-name "$dispatcher" --output json | jq -r '.AccessKeyMetadata[].AccessKeyId')
for key in "${keys[@]}"; do
  aws iam delete-access-key --user-name "$dispatcher" --access-key-id "$key"
done
aws cloudformation delete-stack --stack-name "$stack"
aws cloudformation wait stack-delete-complete --stack-name "$stack"
# The template retains ECR repositories. This refuses to delete a nonempty one.
aws ecr delete-repository --repository-name scope-vcs/staging/checks > "$output/deleted-ecr.json"

# Remove access last. A failed self-deletion must still leave no usable role.
aws iam get-role --role-name "$role" > "$output/role-before.json"
test "$(aws iam list-role-policies --role-name "$role" --query 'length(PolicyNames)' --output text)" = 0
test "$(aws iam list-attached-role-policies --role-name "$role" --query 'length(AttachedPolicies)' --output text)" = 1
test "$(aws iam list-attached-role-policies --role-name "$role" --query 'AttachedPolicies[0].PolicyArn' --output text)" = "$admin_policy"
jq '.Role.AssumeRolePolicyDocument | .Statement[].Condition.DateLessThan["aws:CurrentTime"] = "2026-09-07T00:00:00Z"' \
  "$output/role-before.json" > "$output/disabled-trust.json"
aws iam update-assume-role-policy --role-name "$role" --policy-document "file://$output/disabled-trust.json"
aws iam detach-role-policy --role-name "$role" --policy-arn "$admin_policy"
role_status=disabled
if aws iam delete-role --role-name "$role"; then
  role_status=deleted
fi
jq -n --arg roleStatus "$role_status" --arg collectedThrough "$ended" \
  --argjson keysRevoked "${#keys[@]}" \
  '{stackDeleted: true, retainedEcrDeleted: true, dispatcherKeysRevoked: $keysRevoked, roleStatus: $roleStatus, collectedThrough: $collectedThrough}' \
  > "$output/cleanup.json"
