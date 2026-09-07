#!/usr/bin/env bash
set -euo pipefail
output=${1:?output directory required}
mkdir -p "$output/tasks" "$output/hosts"
cluster=scope-vcs-staging-runner
stack=scope-cloud-runner-staging-mi-experiment
aws cloudformation describe-stacks --stack-name "$stack" > "$output/stack.json"
aws cloudformation list-stack-resources --stack-name "$stack" > "$output/resources.json"
vpc=$(jq -er '.StackResourceSummaries[] | select(.ResourceType == "AWS::EC2::VPC") | .PhysicalResourceId' "$output/resources.json")
nat=$(jq -er '.StackResourceSummaries[] | select(.ResourceType == "AWS::EC2::NatGateway") | .PhysicalResourceId' "$output/resources.json")
started=$(date -u +%FT%TZ)
deadline=$((SECONDS + 2700))
while (( SECONDS < deadline )); do
  now=$(date -u +%FT%TZ)
  # Include recently stopped tasks so even short bootstrap failures are recorded.
  for status in RUNNING STOPPED; do
    aws ecs list-tasks --cluster "$cluster" --desired-status "$status" > "$output/list-$status.json"
    mapfile -t tasks < <(jq -r '.taskArns[]' "$output/list-$status.json")
    if (( ${#tasks[@]} )); then
      aws ecs describe-tasks --cluster "$cluster" --tasks "${tasks[@]}" --include TAGS > "$output/current-tasks-$status.json"
      jq -c --arg at "$now" '{at: $at, tasks, failures}' "$output/current-tasks-$status.json" >> "$output/task-timeline.jsonl"
      while IFS= read -r task; do
        id=$(jq -r '.taskArn | split("/")[-1]' <<< "$task")
        printf '%s\n' "$task" > "$output/tasks/$id.json"
      done < <(jq -c '.tasks[]' "$output/current-tasks-$status.json")
    fi
  done
  aws ec2 describe-instances --filters "Name=vpc-id,Values=$vpc" > "$output/current-hosts.json"
  jq -c --arg at "$now" '{at: $at, instances: [.Reservations[].Instances[]]}' "$output/current-hosts.json" >> "$output/host-timeline.jsonl"
  while IFS= read -r host; do
    id=$(jq -r '.InstanceId' <<< "$host")
    if [[ ! -e "$output/hosts/$id.json" ]]; then
      printf '%s\n' "$host" > "$output/hosts/$id.json"
      echo "$now discovered host $id $(jq -r '.InstanceType' <<< "$host")"
    fi
    mapfile -t volumes < <(jq -r '.BlockDeviceMappings[].Ebs.VolumeId' <<< "$host")
    if (( ${#volumes[@]} )) && [[ ! -e "$output/hosts/$id-volumes.json" ]]; then
      aws ec2 describe-volumes --volume-ids "${volumes[@]}" > "$output/hosts/$id-volumes.json"
    fi
  done < <(jq -c '.Reservations[].Instances[]' "$output/current-hosts.json")
  sleep 5
done
ended=$(date -u +%FT%TZ)
for metric in BytesInFromSource BytesInFromDestination; do
  aws cloudwatch get-metric-statistics --namespace AWS/NATGateway --metric-name "$metric" \
    --dimensions "Name=NatGatewayId,Value=$nat" --start-time "$started" --end-time "$ended" \
    --period 60 --statistics Sum > "$output/nat-$metric.json"
done
aws logs filter-log-events --log-group-name /scope-vcs/staging/cloud-runner \
  --start-time "$(date -d "$started" +%s)000" > "$output/runner-logs.json"
