"""Policy contracts for account security controls. Run with python3 and PyYAML."""
import fnmatch
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
import unittest
import yaml

ROOT = Path(__file__).resolve().parent


class Loader(yaml.SafeLoader):
    pass


def intrinsic(loader, tag, node):
    if isinstance(node, yaml.ScalarNode):
        value = loader.construct_scalar(node)
    elif isinstance(node, yaml.SequenceNode):
        value = loader.construct_sequence(node)
    else:
        value = loader.construct_mapping(node)
    return {tag if tag == 'Ref' else 'Fn::' + tag: value}


Loader.add_multi_constructor('!', intrinsic)


def load(name):
    return yaml.load((ROOT / name).read_text(), Loader=Loader)


def actions(statement):
    value = statement['Action']
    return [value] if isinstance(value, str) else value


class SecurityContracts(unittest.TestCase):
    def security_plan(self, exists, overrides=None):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            log = directory / 'aws-calls.jsonl'
            fake_aws = directory / 'aws'
            fake_aws.write_text(f"#!{sys.executable}\n" + r"""import json, os, sys
with open(os.environ['FAKE_AWS_CALLS'], 'a') as log:
    log.write(json.dumps(sys.argv[1:]) + '\n')
args = sys.argv[3:]
if args[:2] == ['sts', 'get-caller-identity']:
    print('arn:aws:sts::123456789012:assumed-role/operator/test')
elif args[:2] == ['cloudformation', 'describe-stacks']:
    sys.exit(0 if os.environ['FAKE_STACK_EXISTS'] == 'true' else 1)
else:
    print('{}')
""")
            fake_aws.chmod(0o755)
            environment = dict(os.environ)
            for key in ['SECURITY_ALERT_EMAIL', 'SECURITY_AUDIT_PRINCIPAL_ARN', 'ENABLE_GUARDDUTY']:
                environment.pop(key, None)
            environment.update(PATH=f"{directory}:{environment['PATH']}", AWS_REGION='us-east-1',
                               FAKE_AWS_CALLS=str(log), FAKE_STACK_EXISTS='true' if exists else 'false')
            environment.update(overrides or {})
            result = subprocess.run(['bash', str(ROOT / 'apply-security-controls.sh'), 'plan'],
                                    env=environment, text=True, capture_output=True)
            calls = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
            creates = [call for call in calls if call[2:4] == ['cloudformation', 'create-change-set']]
            return result, creates

    def test_security_update_preserves_all_unset_parameters(self):
        result, creates = self.security_plan(True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(creates), 1)
        for name in ['AlertEmail', 'AuditPrincipalArn', 'EnableGuardDuty']:
            self.assertIn(f'ParameterKey={name},UsePreviousValue=true', creates[0])

    def test_security_update_honors_explicit_values_including_cleared_audit_principal(self):
        result, creates = self.security_plan(True, {
            'SECURITY_ALERT_EMAIL': 'security@example.com',
            'SECURITY_AUDIT_PRINCIPAL_ARN': '',
            'ENABLE_GUARDDUTY': 'true',
        })
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('ParameterKey=AlertEmail,ParameterValue=security@example.com', creates[0])
        self.assertIn('ParameterKey=AuditPrincipalArn,ParameterValue=', creates[0])
        self.assertIn('ParameterKey=EnableGuardDuty,ParameterValue=true', creates[0])

    def test_security_create_requires_email_and_uses_safe_optional_defaults(self):
        self.assertNotIn('Default', load('security-controls.yaml')['Parameters']['AlertEmail'])
        for overrides in [{}, {'SECURITY_ALERT_EMAIL': ''}]:
            result, creates = self.security_plan(False, overrides)
            self.assertEqual(result.returncode, 2)
            self.assertIn('SECURITY_ALERT_EMAIL is required', result.stderr)
            self.assertEqual(creates, [])
        result, creates = self.security_plan(False, {'SECURITY_ALERT_EMAIL': 'security@example.com'})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('ParameterKey=AuditPrincipalArn,ParameterValue=', creates[0])
        self.assertIn('ParameterKey=EnableGuardDuty,ParameterValue=false', creates[0])

    def test_durable_management_logging(self):
        resources = load('security-controls.yaml')['Resources']
        bucket = resources['AuditBucket']
        self.assertEqual(bucket['DeletionPolicy'], 'Retain')
        self.assertEqual(bucket['UpdateReplacePolicy'], 'Retain')
        props = bucket['Properties']
        self.assertTrue(all(props['PublicAccessBlockConfiguration'].values()))
        self.assertEqual(props['VersioningConfiguration']['Status'], 'Enabled')
        self.assertTrue(props['BucketEncryption']['ServerSideEncryptionConfiguration'])
        trail = resources['ManagementTrail']['Properties']
        for key in ['IsLogging', 'IsMultiRegionTrail', 'IncludeGlobalServiceEvents', 'EnableLogFileValidation']:
            self.assertIs(trail[key], True)
        self.assertEqual(trail['EventSelectors'], [{'IncludeManagementEvents': True, 'ReadWriteType': 'All'}])

    def test_alert_publish_is_bound_to_rules(self):
        resources = load('security-controls.yaml')['Resources']
        statements = resources['AlertTopicPolicy']['Properties']['PolicyDocument']['Statement']
        sids = [statement.get('Sid') for statement in statements]
        self.assertTrue(all(sids), 'SNS statements require an explicit ID')
        self.assertEqual(len(sids), len(set(sids)), 'SNS statement IDs must be unique')
        statement = statements[0]
        self.assertEqual(statement['Principal'], {'Service': 'events.amazonaws.com'})
        self.assertEqual(statement['Condition']['ArnEquals']['aws:SourceArn'], [{'Fn::GetAtt': 'RootActivityRule.Arn'}, {'Fn::GetAtt': 'GuardDutyFindingRule.Arn'}])
        self.assertIn('AWS Console Sign In via CloudTrail', resources['RootActivityRule']['Properties']['EventPattern']['detail-type'])
        self.assertEqual(resources['IamMutationRule']['Properties']['EventPattern']['detail']['readOnly'], [False, 'false'])

    def test_root_authentication_is_direct_but_iam_changes_are_aggregated(self):
        resources = load('security-controls.yaml')['Resources']
        root = resources['RootActivityRule']['Properties']['EventPattern']
        self.assertEqual(root['source'], ['aws.signin'])
        self.assertEqual(root['detail']['eventSource'], ['signin.amazonaws.com'])
        self.assertEqual(root['detail']['eventName'], ['ConsoleLogin', 'AuthorizeOAuth2Access'])
        self.assertEqual(root['detail']['userIdentity']['type'], ['Root'])
        self.assertNotIn('CreateOAuth2Token', root['detail']['eventName'])
        iam = resources['IamMutationRule']['Properties']
        self.assertEqual(iam['EventPattern']['detail']['readOnly'], [False, 'false'])
        self.assertEqual(len(iam['Targets']), 1)
        self.assertIn(':log-group:/aws/events/scope-security-iam-mutations', iam['Targets'][0]['Arn']['Fn::Sub'])
        metric = resources['IamMutationMetric']['Properties']
        self.assertEqual(metric['LogGroupName'], {'Ref': 'IamMutationLogGroup'})
        self.assertEqual(metric['FilterPattern'], '{ $.detail.eventSource = "iam.amazonaws.com" }')
        self.assertEqual(metric['MetricTransformations'][0]['MetricValue'], '1')
        alarm = resources['IamMutationAlarm']['Properties']
        self.assertEqual(alarm['Namespace'], metric['MetricTransformations'][0]['MetricNamespace'])
        self.assertEqual(alarm['MetricName'], metric['MetricTransformations'][0]['MetricName'])
        self.assertEqual(alarm['Period'], 900)
        self.assertEqual(alarm['Statistic'], 'Sum')
        self.assertEqual(alarm['Threshold'], 1)
        self.assertEqual(alarm['TreatMissingData'], 'notBreaching')
        self.assertEqual(alarm['AlarmActions'], [{'Ref': 'AlertTopic'}])
        self.assertNotIn('OKActions', alarm)
        self.assertNotIn('InsufficientDataActions', alarm)
        topic_statements = resources['AlertTopicPolicy']['Properties']['PolicyDocument']['Statement']
        grant = next(s for s in topic_statements if s['Sid'] == 'AllowIamMutationSummary')
        self.assertEqual(grant['Principal'], {'Service': 'cloudwatch.amazonaws.com'})
        self.assertEqual(grant['Condition']['ArnEquals']['aws:SourceArn']['Fn::Sub'], 'arn:${AWS::Partition}:cloudwatch:${AWS::Region}:${AWS::AccountId}:alarm:scope-security-iam-mutations')
        self.assertEqual(grant['Condition']['StringEquals']['aws:SourceAccount'], {'Ref': 'AWS::AccountId'})

    def testRecoveryAlarmPublishesOnlyFromExactAccountAndAlarm(self):
        resources = load('security-controls.yaml')['Resources']
        statements = resources['AlertTopicPolicy']['Properties']['PolicyDocument']['Statement']
        grant = next(s for s in statements if s['Sid'] == 'AllowRecoverySetHealthAlarm')
        self.assertEqual(grant['Principal'], {'Service': 'cloudwatch.amazonaws.com'})
        self.assertEqual(grant['Action'], 'sns:Publish')
        self.assertEqual(grant['Resource'], {'Ref': 'AlertTopic'})
        self.assertEqual(grant['Condition']['StringEquals'], {'aws:SourceAccount': {'Ref': 'AWS::AccountId'}})
        self.assertEqual(grant['Condition']['ArnEquals']['aws:SourceArn']['Fn::Sub'], 'arn:${AWS::Partition}:cloudwatch:${AWS::Region}:${AWS::AccountId}:alarm:scope-production-recovery-set-health')

    def test_paid_monitoring_is_opt_in_and_audit_never_reads_content(self):
        template = load('security-controls.yaml')
        self.assertEqual(template['Parameters']['EnableGuardDuty']['Default'], 'false')
        role = template['Resources']['RoutineAuditRole']
        self.assertEqual(role['Condition'], 'CreateAuditRole')
        trust = role['Properties']['AssumeRolePolicyDocument']['Statement'][0]
        self.assertEqual(trust['Principal']['AWS'], {'Ref': 'AuditPrincipalArn'})
        statements = next(policy for policy in role['Properties']['Policies'] if policy['PolicyName'] == 'metadata-only-audit')['PolicyDocument']['Statement']
        for forbidden in ['secretsmanager:GetSecretValue', 'ssm:GetParameter', 'kms:Decrypt', 's3:GetObject', 'logs:GetLogEvents', 'lambda:GetFunctionConfiguration', 'ecs:DescribeTaskDefinition', 'sts:AssumeRole']:
            self.assertTrue(any(s['Effect'] == 'Deny' and any(fnmatch.fnmatchcase(forbidden, p) for p in actions(s)) for s in statements), forbidden)
            self.assertFalse(any(s['Effect'] == 'Allow' and any(fnmatch.fnmatchcase(forbidden, p) for p in actions(s)) for s in statements), forbidden)

    def test_human_can_only_enroll_mfa_before_assuming_audit(self):
        resources = load('human-access.yaml')['Resources']
        self.assertEqual(list(resources), ['HumanUser'])
        user = resources['HumanUser']['Properties']
        self.assertNotIn('LoginProfile', user)
        self.assertNotIn('ManagedPolicyArns', user)
        statements = user['Policies'][0]['PolicyDocument']['Statement']
        pre_mfa = next(s for s in statements if s.get('Sid') == 'DenyBeforeMFAExceptEnrollment')
        self.assertEqual(pre_mfa['Condition'], {'BoolIfExists': {'aws:MultiFactorAuthPresent': 'false'}})
        self.assertEqual(set(pre_mfa['NotAction']), {'iam:EnableMFADevice', 'iam:GetUser', 'iam:ListMFADevices', 'iam:ResyncMFADevice'})
        allow = next(s for s in statements if s.get('Sid') == 'AssumeOnlyAudit')
        self.assertEqual(allow['Resource']['Fn::Sub'], 'arn:${AWS::Partition}:iam::${AWS::AccountId}:role/scope-security-audit')
        self.assertEqual(allow['Condition'], {'Bool': {'aws:MultiFactorAuthPresent': 'true'}})
        for statement in statements:
            if statement['Effect'] == 'Allow':
                for action in actions(statement):
                    self.assertTrue(action.startswith('iam:') or action == 'sts:AssumeRole')
        deny = next(s for s in statements if s.get('Sid') == 'NoPermanentProgrammaticCredentialsOrMfaRemoval')
        self.assertIn('iam:CreateAccessKey', deny['Action'])
        self.assertIn('iam:DeactivateMFADevice', deny['Action'])
        role = load('security-controls.yaml')['Resources']['RoutineAuditRole']['Properties']
        trust = role['AssumeRolePolicyDocument']['Statement'][0]
        self.assertEqual(trust['Condition'], {'Bool': {'aws:MultiFactorAuthPresent': 'true'}})
        login = next(policy for policy in role['Policies'] if policy['PolicyName'] == 'login-to-audit-session')['PolicyDocument']['Statement'][0]
        self.assertEqual(set(login['Action']), {'signin:AuthorizeOAuth2Access', 'signin:CreateOAuth2Token'})
        self.assertEqual({r['Fn::Sub'] for r in login['Resource']}, {
            'arn:${AWS::Partition}:signin:us-east-1:${AWS::AccountId}:oauth2/public-client/localhost',
            'arn:${AWS::Partition}:signin:us-east-1:${AWS::AccountId}:oauth2/public-client/remote',
        })

    def test_ci_cannot_change_its_permissions_or_unrelated_stacks(self):
        role = load('cloud-runner.yaml')['Resources']['GitHubInfrastructureRole']['Properties']
        self.assertNotIn('ManagedPolicyArns', role)
        trust = role['AssumeRolePolicyDocument']['Statement'][0]['Condition']['StringEquals']
        self.assertIn(':environment:AWS-infrastructure', trust['token.actions.githubusercontent.com:sub']['Fn::Sub'][0])
        self.assertEqual(trust['token.actions.githubusercontent.com:ref'], 'refs/heads/main')
        self.assertEqual(trust['token.actions.githubusercontent.com:job_workflow_ref']['Fn::Sub'], '${GitHubRepository}/.github/workflows/scope-aws-infrastructure-execute.yml@refs/heads/main')
        statements = role['Policies'][0]['PolicyDocument']['Statement']
        allowed = [action for statement in statements for action in actions(statement)]
        self.assertFalse(any(action.startswith('iam:') and action != 'iam:PassRole' for action in allowed))
        for statement in statements:
            if statement['Resource'] == '*':
                self.assertEqual(actions(statement), ['cloudformation:ValidateTemplate'])

    def test_protected_workflow_uses_exact_reusable_execution(self):
        workflow_root = ROOT.parent.parent / '.github' / 'workflows'
        caller = (workflow_root / 'scope-aws-infrastructure.yml').read_text()
        execution = (workflow_root / 'scope-aws-infrastructure-execute.yml').read_text()
        self.assertIn("if: github.ref == 'refs/heads/main'", caller)
        self.assertIn('uses: ./.github/workflows/scope-aws-infrastructure-execute.yml', caller)
        self.assertIn('environment: AWS-infrastructure', execution)
        self.assertIn('SCOPE_AWS_EXECUTION_ROLE_ARN: ${{ vars.SCOPE_AWS_EXECUTION_ROLE_ARN }}', execution)
        self.assertIn('deploy/aws/apply-cloud-runner.sh "$COMMAND" "$CHANGE_SET_ARN"', execution)

    def test_broker_code_versions_are_private_retained_and_readable_by_execution(self):
        resources = load('security-deployment-role.yaml')['Resources']
        bucket = resources['BrokerArtifactBucket']
        self.assertEqual(bucket['DeletionPolicy'], 'Retain')
        self.assertEqual(bucket['UpdateReplacePolicy'], 'Retain')
        props = bucket['Properties']
        self.assertTrue(all(props['PublicAccessBlockConfiguration'].values()))
        self.assertEqual(props['VersioningConfiguration'], {'Status': 'Enabled'})
        self.assertEqual(props['OwnershipControls']['Rules'], [{'ObjectOwnership': 'BucketOwnerEnforced'}])
        self.assertTrue(props['BucketEncryption']['ServerSideEncryptionConfiguration'])
        tls = resources['BrokerArtifactBucketPolicy']['Properties']['PolicyDocument']['Statement'][0]
        self.assertEqual(tls['Effect'], 'Deny')
        self.assertEqual(tls['Condition'], {'Bool': {'aws:SecureTransport': 'false'}})
        statements = resources['InfrastructureExecutionRole']['Properties']['Policies'][0]['PolicyDocument']['Statement']
        readers = [statement for statement in statements if 's3:GetObjectVersion' in actions(statement)]
        self.assertEqual(len(readers), 1)
        self.assertEqual(readers[0]['Resource']['Fn::Sub'], 'arn:${AWS::Partition}:s3:::scope-dispatch-broker-artifacts-${AWS::AccountId}-${AWS::Region}/*')
        self.assertFalse(any(statement['Effect'] == 'Allow' and any(action.startswith('s3:Put') for action in actions(statement)) for statement in statements))

    def test_runtime_boundary_cannot_be_removed_or_changed_by_execution(self):
        resources = load('security-deployment-role.yaml')['Resources']
        statements = resources['InfrastructureExecutionRole']['Properties']['Policies'][0]['PolicyDocument']['Statement']
        self.assertTrue(any(s['Effect'] == 'Deny' and 'iam:DeleteRolePermissionsBoundary' in actions(s) for s in statements))
        for statement in statements:
            if statement['Effect'] == 'Allow' and 'iam:CreateRole' in actions(statement):
                self.assertIn('scope-runtime-boundary', json.dumps(statement['Condition']))
            if statement['Effect'] == 'Allow' and 'iam:PassRole' in actions(statement):
                self.assertNotEqual(statement['Resource'], '*')
                self.assertIn('iam:PassedToService', statement['Condition']['StringEquals'])
        protected = next(s for s in statements if s.get('Sid') == 'ProtectTrustAndPermissionOwners')
        for name in ['scope-infrastructure-execution', 'scope-security-', 'github', 'oidc-provider', 'scope-runtime-boundary']:
            self.assertIn(name, json.dumps(protected['Resource']))
        boundary = resources['RuntimeBoundary']['Properties']['PolicyDocument']['Statement']
        for statement in boundary:
            if 'Fn::If' in statement:
                continue
            self.assertEqual(statement['Effect'], 'Allow')
            self.assertFalse(any(a.startswith('iam:') and a != 'iam:PassRole' for a in actions(statement)))


if __name__ == '__main__':
    unittest.main()
