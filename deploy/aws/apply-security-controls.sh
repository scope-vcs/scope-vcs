#!/usr/bin/env bash
set -euo pipefail
export AWS_PAGER=""
command_name="${1:-}"
change_set_arn="${2:-}"
case "$command_name" in
  validate|plan) [[ -z "$change_set_arn" ]] || exit 2 ;;
  apply) [[ -n "$change_set_arn" ]] || { echo 'apply requires an exact reviewed change-set ARN' >&2; exit 2; } ;;
  *) echo "usage: $0 <validate|plan|apply> [reviewed change-set ARN]" >&2; exit 2 ;;
esac
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
aws_region="${AWS_REGION:-us-east-1}"
[[ "$aws_region" == us-east-1 ]] || { echo 'security controls belong in us-east-1 to capture global IAM events' >&2; exit 2; }
stack_name=scope-security-controls
aws_command() { aws --region "$aws_region" "$@"; }
aws_command cloudformation validate-template --template-body "file://$script_dir/security-controls.yaml"
[[ "$command_name" != validate ]] || exit 0
caller_arn="$(aws_command sts get-caller-identity --query Arn --output text)"
[[ "$caller_arn" != *:root ]] || { echo 'use a non-root temporary administration session' >&2; exit 1; }
if [[ "$command_name" == apply ]]; then
  aws_command cloudformation describe-change-set --stack-name "$stack_name" --change-set-name "$change_set_arn" --output table
  aws_command cloudformation execute-change-set --stack-name "$stack_name" --change-set-name "$change_set_arn"
  exit 0
fi
operation=CREATE
if aws_command cloudformation describe-stacks --stack-name "$stack_name" >/dev/null 2>&1; then operation=UPDATE; fi
if [[ "$operation" == CREATE && -z "${SECURITY_ALERT_EMAIL:-}" ]]; then
  echo 'SECURITY_ALERT_EMAIL is required when creating security controls' >&2
  exit 2
fi
parameters=()
append_parameter() {
  local parameter_name="$1" environment_name="$2" create_default="$3"
  if [[ -v "$environment_name" ]]; then
    parameters+=("ParameterKey=$parameter_name,ParameterValue=${!environment_name}")
  elif [[ "$operation" == UPDATE ]]; then
    parameters+=("ParameterKey=$parameter_name,UsePreviousValue=true")
  else
    parameters+=("ParameterKey=$parameter_name,ParameterValue=$create_default")
  fi
}
append_parameter AlertEmail SECURITY_ALERT_EMAIL ''
append_parameter AuditPrincipalArn SECURITY_AUDIT_PRINCIPAL_ARN ''
append_parameter EnableGuardDuty ENABLE_GUARDDUTY false
aws_command cloudformation create-change-set \
  --stack-name "$stack_name" \
  --change-set-name "security-$(date -u +%Y%m%dT%H%M%SZ)" \
  --change-set-type "$operation" \
  --template-body "file://$script_dir/security-controls.yaml" \
  --capabilities CAPABILITY_NAMED_IAM \
  --parameters "${parameters[@]}" \
  --query Id --output text
