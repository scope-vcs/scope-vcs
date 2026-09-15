#!/usr/bin/env bash
set -euo pipefail
export AWS_PAGER=""
command_name="${1:-}"
change_set_arn="${2:-}"
case "$command_name" in
  validate|plan) [[ -z "$change_set_arn" ]] || exit 2 ;;
  apply) [[ -n "$change_set_arn" ]] || { echo 'broker apply requires an exact reviewed change-set ARN' >&2; exit 2; } ;;
  *) echo "usage: $0 <validate|plan|apply> [reviewed change-set ARN]" >&2; exit 2 ;;
esac
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
aws_region="${AWS_REGION:-us-east-1}"
[[ "$aws_region" == us-east-1 ]] || { echo 'the production broker belongs in us-east-1' >&2; exit 2; }
stack_name=scope-dispatch-broker-production
aws_command() { aws --region "$aws_region" "$@"; }
aws_command cloudformation validate-template --template-body "file://$script_dir/dispatch-broker.yaml" --query Description --output text
[[ "$command_name" != validate ]] || exit 0
read -r caller_arn account_id < <(aws_command sts get-caller-identity --query '[Arn,Account]' --output text)
[[ "$caller_arn" != *:root ]] || { echo 'use the protected infrastructure role' >&2; exit 1; }
execution_role_arn="${SCOPE_AWS_EXECUTION_ROLE_ARN:-}"
[[ "$execution_role_arn" == "arn:aws:iam::$account_id:role/scope-infrastructure-execution" ]] || {
  echo 'SCOPE_AWS_EXECUTION_ROLE_ARN must name the account protected execution role' >&2; exit 2;
}
if [[ "$command_name" == plan ]]; then
  [[ "${SCOPE_BROKER_CODE_BUCKET:-}" == "scope-dispatch-broker-artifacts-$account_id-$aws_region" ]] || {
    echo 'broker plan requires the protected artifact bucket' >&2; exit 2;
  }
  [[ "${SCOPE_BROKER_CODE_KEY:-}" =~ ^[A-Za-z0-9._/-]+$ ]] || {
    echo 'broker plan requires a reviewed ZIP object key' >&2; exit 2;
  }
  [[ "${SCOPE_BROKER_CODE_VERSION:-}" =~ ^[A-Za-z0-9._+/=-]+$ && "$SCOPE_BROKER_CODE_VERSION" != null ]] || {
    echo 'broker plan requires an immutable non-null S3 object version' >&2; exit 2;
  }
  status="$(aws_command cloudformation describe-stacks --stack-name "$stack_name" --query 'Stacks[0].StackStatus' --output text)"
  case "$status" in
    CREATE_COMPLETE|UPDATE_COMPLETE|UPDATE_ROLLBACK_COMPLETE) ;;
    *) echo 'the broker must already be bootstrapped and ready for update' >&2; exit 1 ;;
  esac
  parameters=()
  # Preserve secret and topology values without reading or accepting replacements.
  for key in Environment ApiUrl DispatchAuthorityToken ClusterArn SubnetIds SecurityGroupId ExecutionRoleArn RunnerLogGroup RegistryCredentialsSecretArn RegistryCredentialsHost; do
    parameters+=("ParameterKey=$key,UsePreviousValue=true")
  done
  parameters+=(
    "ParameterKey=CodeBucket,ParameterValue=$SCOPE_BROKER_CODE_BUCKET"
    "ParameterKey=CodeKey,ParameterValue=$SCOPE_BROKER_CODE_KEY"
    "ParameterKey=CodeVersion,ParameterValue=$SCOPE_BROKER_CODE_VERSION"
  )
  change_set_arn="$(aws_command cloudformation create-change-set \
    --stack-name "$stack_name" --change-set-name "broker-$(date -u +%Y%m%dT%H%M%SZ)-$$" \
    --change-set-type UPDATE --template-body "file://$script_dir/dispatch-broker.yaml" \
    --capabilities CAPABILITY_NAMED_IAM --role-arn "$execution_role_arn" \
    --parameters "${parameters[@]}" --description 'Reviewed immutable broker code update' --query Id --output text)"
  if ! aws_command cloudformation wait change-set-create-complete --stack-name "$stack_name" --change-set-name "$change_set_arn"; then
    reason="$(aws_command cloudformation describe-change-set --stack-name "$stack_name" --change-set-name "$change_set_arn" --query StatusReason --output text)"
    if [[ "$reason" == *"didn't contain changes"* || "$reason" == *"No updates are to be performed"* ]]; then
      echo 'No broker infrastructure changes.'
      exit 0
    fi
    echo 'broker change-set creation failed; inspect its CloudFormation status' >&2
    exit 1
  fi
else
  [[ "$change_set_arn" =~ ^arn:aws:cloudformation:us-east-1:${account_id}:changeSet/[A-Za-z][-A-Za-z0-9]*/[A-Za-z0-9-]+$ ]] || {
    echo 'broker apply requires an exact change-set ARN in this account and region' >&2; exit 2;
  }
  [[ -z "${SCOPE_BROKER_CODE_BUCKET:-}${SCOPE_BROKER_CODE_KEY:-}${SCOPE_BROKER_CODE_VERSION:-}" ]] || {
    echo 'broker apply uses only the reviewed change set; omit code inputs' >&2; exit 2;
  }
fi
# Print resource changes only. Parameter values and templates can contain secrets.
aws_command cloudformation describe-change-set --stack-name "$stack_name" --change-set-name "$change_set_arn" \
  --query 'Changes[].ResourceChange.[Action,LogicalResourceId,ResourceType,Replacement]' --output table
if [[ "$command_name" == plan ]]; then
  printf 'Change set: %s\n' "$change_set_arn"
  printf 'Apply the reviewed plan with: %q apply %q\n' "$0" "$change_set_arn"
  exit 0
fi
aws_command cloudformation execute-change-set --stack-name "$stack_name" --change-set-name "$change_set_arn"
aws_command cloudformation wait stack-update-complete --stack-name "$stack_name"
echo 'Broker stack update complete.'
