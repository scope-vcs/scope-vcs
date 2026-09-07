#!/usr/bin/env bash
set -euo pipefail

cluster_arn="${1:?usage: collect-managed-task-evidence.sh CLUSTER_ARN TASK_ARN OUTPUT_JSON}"
task_arn="${2:?usage: collect-managed-task-evidence.sh CLUSTER_ARN TASK_ARN OUTPUT_JSON}"
output_path="${3:?usage: collect-managed-task-evidence.sh CLUSTER_ARN TASK_ARN OUTPUT_JSON}"
aws_region="${AWS_REGION:-us-east-1}"

if [[ -e "$output_path" ]]; then
  echo "refusing to overwrite $output_path" >&2
  exit 1
fi

umask 077
work_dir=$(mktemp -d)
cleanup() {
  rm -rf -- "$work_dir"
}
trap cleanup EXIT

observed_at=$(date -u +%FT%TZ)
container_instance_arn=""
deadline=$((SECONDS + 3600))

while (( SECONDS < deadline )); do
  aws ecs describe-tasks \
    --region "$aws_region" \
    --cluster "$cluster_arn" \
    --tasks "$task_arn" \
    --include TAGS > "$work_dir/task.json"
  container_instance_arn=$(jq -r '.tasks[0].containerInstanceArn // empty' "$work_dir/task.json")
  [[ -n "$container_instance_arn" ]] && break
  sleep 2
done

if [[ -z "$container_instance_arn" ]]; then
  echo "task did not receive a container instance within one hour" >&2
  exit 1
fi

aws ecs describe-container-instances \
  --region "$aws_region" \
  --cluster "$cluster_arn" \
  --container-instances "$container_instance_arn" \
  --include TAGS > "$work_dir/container-instance.json"
ec2_instance_id=$(jq -er '.containerInstances[0].ec2InstanceId' "$work_dir/container-instance.json")
aws ec2 describe-instances \
  --region "$aws_region" \
  --instance-ids "$ec2_instance_id" > "$work_dir/ec2-instance.json"
volume_ids=$(jq -r '.Reservations[0].Instances[0].BlockDeviceMappings[].Ebs.VolumeId' "$work_dir/ec2-instance.json")
readarray -t volumes <<< "$volume_ids"
aws ec2 describe-volumes --region "$aws_region" --volume-ids "${volumes[@]}" > "$work_dir/volumes.json"

while (( SECONDS < deadline )); do
  aws ecs describe-tasks \
    --region "$aws_region" \
    --cluster "$cluster_arn" \
    --tasks "$task_arn" \
    --include TAGS > "$work_dir/task.json"
  [[ "$(jq -r '.tasks[0].lastStatus // empty' "$work_dir/task.json")" == "STOPPED" ]] && break
  sleep 5
done

if [[ "$(jq -r '.tasks[0].lastStatus // empty' "$work_dir/task.json")" != "STOPPED" ]]; then
  echo "task did not stop within one hour" >&2
  exit 1
fi

host_deadline=$((SECONDS + 900))
last_running_observed_at=""
first_non_running_observed_at=""
host_state=""
while (( SECONDS < host_deadline )); do
  aws ec2 describe-instances \
    --region "$aws_region" \
    --instance-ids "$ec2_instance_id" > "$work_dir/ec2-final.json"
  host_observed_at=$(date -u +%FT%TZ)
  host_state=$(jq -er '.Reservations[0].Instances[0].State.Name' "$work_dir/ec2-final.json")
  if [[ "$host_state" == running ]]; then
    last_running_observed_at="$host_observed_at"
  elif [[ -z "$first_non_running_observed_at" ]]; then
    first_non_running_observed_at="$host_observed_at"
  fi
  [[ "$host_state" == terminated ]] && break
  sleep 5
done

jq -n \
  --arg observedAt "$observed_at" \
  --arg lastRunningObservedAt "$last_running_observed_at" \
  --arg firstNonRunningObservedAt "$first_non_running_observed_at" \
  --arg finalHostObservedAt "$host_observed_at" \
  --argjson task "$(<"$work_dir/task.json")" \
  --argjson containerInstance "$(<"$work_dir/container-instance.json")" \
  --argjson ec2Instance "$(<"$work_dir/ec2-instance.json")" \
  --argjson ec2Final "$(<"$work_dir/ec2-final.json")" \
  --argjson volumes "$(<"$work_dir/volumes.json")" \
  '{
    observedAt: $observedAt,
    task: $task.tasks[0],
    taskFailures: $task.failures,
    containerInstance: $containerInstance.containerInstances[0],
    containerInstanceFailures: $containerInstance.failures,
    ec2Instance: $ec2Instance.Reservations[0].Instances[0],
    volumes: $volumes.Volumes,
    lastRunningObservedAt: $lastRunningObservedAt,
    firstNonRunningObservedAt: $firstNonRunningObservedAt,
    finalHostObservedAt: $finalHostObservedAt,
    finalEc2Instance: $ec2Final.Reservations[0].Instances[0]
  }' > "$output_path"
echo "$output_path"
if [[ "$host_state" != terminated ]]; then
  echo "host did not terminate within 15 minutes of task completion; partial evidence saved" >&2
  exit 1
fi
