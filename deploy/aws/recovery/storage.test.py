"""Recovery trust and retention contracts. Run with python3 and PyYAML."""
import fnmatch
from pathlib import Path
import re
import unittest

import yaml


class Loader(yaml.SafeLoader):
    pass


def intrinsic(loader, tag, node):
    if isinstance(node, yaml.ScalarNode):
        value = loader.construct_scalar(node)
    elif isinstance(node, yaml.SequenceNode):
        value = loader.construct_sequence(node)
    else:
        value = loader.construct_mapping(node)
    return {tag: value}


Loader.add_multi_constructor('!', intrinsic)
TEMPLATE = yaml.load(Path(__file__).with_name('storage.yaml').read_text(), Loader=Loader)
RESOURCES = TEMPLATE['Resources']


def denied(statement, action):
    return statement['Effect'] == 'Deny' and not any(
        fnmatch.fnmatchcase(action, allowed) for allowed in statement['NotAction']
    )


class RecoveryStorageContracts(unittest.TestCase):
    def testDurablePrivateRecoverableArchives(self):
        bucket = RESOURCES['RecoveryBucket']
        self.assertEqual(bucket['DeletionPolicy'], 'Retain')
        self.assertEqual(bucket['UpdateReplacePolicy'], 'Retain')
        props = bucket['Properties']
        self.assertTrue(all(props['PublicAccessBlockConfiguration'].values()))
        self.assertEqual(props['OwnershipControls']['Rules'], [{'ObjectOwnership': 'BucketOwnerEnforced'}])
        self.assertEqual(props['VersioningConfiguration']['Status'], 'Enabled')
        self.assertTrue(props['ObjectLockEnabled'])
        self.assertEqual(props['ObjectLockConfiguration']['Rule']['DefaultRetention'], {'Mode': 'GOVERNANCE', 'Days': 35})
        self.assertEqual(props['BucketEncryption']['ServerSideEncryptionConfiguration'], [
            {'ServerSideEncryptionByDefault': {'SSEAlgorithm': 'AES256'}}
        ])
        lifecycle = props['LifecycleConfiguration']['Rules'][0]
        self.assertEqual(lifecycle['ExpirationInDays'], 42)
        self.assertEqual(lifecycle['NoncurrentVersionExpiration']['NoncurrentDays'], 42)
        self.assertEqual(lifecycle['AbortIncompleteMultipartUpload']['DaysAfterInitiation'], 1)

    def testTrustRequiresMainRecoveryWorkflowAndSeparateReader(self):
        parameter = TEMPLATE['Parameters']['ReaderPrincipalArn']
        self.assertNotIn('Default', parameter)
        pattern = parameter['AllowedPattern']
        self.assertIsNotNone(re.fullmatch(pattern, 'arn:aws:iam::123456789012:user/scope-operator'))
        for invalid in ['*', 'arn:aws:iam::123456789012:root', 'arn:aws:iam::123456789012:role/*', 'arn:aws:sts::123456789012:assumed-role/operator/session']:
            self.assertIsNone(re.fullmatch(pattern, invalid))
        reader = RESOURCES['RecoveryReaderRole']['Properties']['AssumeRolePolicyDocument']['Statement']
        self.assertEqual(reader, [{'Effect': 'Allow', 'Principal': {'AWS': {'Ref': 'ReaderPrincipalArn'}}, 'Action': 'sts:AssumeRole', 'Condition': {'Bool': {'aws:MultiFactorAuthPresent': 'true'}, 'StringEquals': {'aws:PrincipalAccount': {'Ref': 'AWS::AccountId'}}}}])
        writer = RESOURCES['RecoveryWriterRole']['Properties']['AssumeRolePolicyDocument']['Statement'][0]
        self.assertEqual(writer['Principal'], {'Federated': {'Ref': 'GitHubOidcProviderArn'}})
        self.assertEqual(writer['Action'], 'sts:AssumeRoleWithWebIdentity')
        conditions = writer['Condition']['StringEquals']
        self.assertEqual(conditions['token.actions.githubusercontent.com:aud'], 'sts.amazonaws.com')
        self.assertEqual(conditions['token.actions.githubusercontent.com:repository_id'], {'Ref': 'GitHubRepositoryId'})
        self.assertEqual(conditions['token.actions.githubusercontent.com:ref'], 'refs/heads/main')
        self.assertEqual(conditions['token.actions.githubusercontent.com:job_workflow_ref'], {'Sub': '${GitHubRepository}/.github/workflows/recovery-execute.yml@refs/heads/main'})
        self.assertEqual(conditions['token.actions.githubusercontent.com:sub']['Sub'][0], 'repo:${Owner}@${GitHubRepositoryOwnerId}/${Repository}@${GitHubRepositoryId}:ref:refs/heads/main')
        self.assertEqual(RESOURCES['RecoveryWriterRole']['Properties']['MaxSessionDuration'], 7200)

    def testReaderLoginHasOnlyExactPublicOAuthClients(self):
        policies = RESOURCES['RecoveryReaderRole']['Properties']['Policies']
        login = next(p for p in policies if p['PolicyName'] == 'login-to-recovery-session')
        statement = login['PolicyDocument']['Statement'][0]
        self.assertEqual(statement['Action'], ['signin:AuthorizeOAuth2Access', 'signin:CreateOAuth2Token'])
        self.assertEqual(statement['Resource'], [{'Sub': 'arn:${AWS::Partition}:signin:us-east-1:${AWS::AccountId}:oauth2/public-client/' + name} for name in ['localhost', 'remote']])

    def testSchedulerUsesOnlyPinnedActionsAndPrivateCapture(self):
        root = Path(__file__).resolve().parents[3]
        schedule = yaml.safe_load((root / '.github/workflows/recovery.yml').read_text())
        execute = yaml.safe_load((root / '.github/workflows/recovery-execute.yml').read_text())
        # PyYAML YAML 1.1 represents GitHub's unquoted on key as True.
        self.assertEqual(schedule[True]['schedule'], [{'cron': '17 7 * * *'}])
        self.assertFalse(schedule['concurrency']['cancel-in-progress'])
        job = execute['jobs']['capture']
        self.assertEqual(job['if'], "github.ref == 'refs/heads/main'")
        for step in job['steps']:
            if 'uses' in step:
                self.assertRegex(step['uses'], r'@[a-f0-9]{40}$')
                self.assertNotIn('upload-artifact', step['uses'])
        installer = next(s for s in job['steps'] if s.get('name') == 'Install verified age binary')
        self.assertIn('age/age age/age-keygen', installer['run'])
        validation = next(s for s in job['steps'] if s.get('name') == 'Verify recovery tooling')
        self.assertIn('command -v age age-keygen', validation['run'])
        capture = next(s for s in job['steps'] if s.get('run') == 'python .github/scripts/recovery-run.py')
        self.assertEqual(capture['env']['RAILWAY_TOKEN'], '${{ secrets.RAILWAY_TOKEN }}')
        self.assertEqual(capture['env']['SCOPE_RAILWAY_SSH_PRIVATE_KEY'], '${{ secrets.SCOPE_RAILWAY_SSH_PRIVATE_KEY }}')
        failure = job['steps'][-1]
        self.assertEqual(failure['if'], "${{ failure() && steps.credentials.outcome == 'success' }}")
        self.assertIn('Value:0', failure['run'])

    def testIdentityGrantsCannotTurnWriterIntoReaderOrDestroyer(self):
        statements = RESOURCES['RecoveryBucketPolicy']['Properties']['PolicyDocument']['Statement']
        writer = next(s for s in statements if s['Sid'] == 'RestrictWriterEvenWithAdditionalIdentityGrants')
        self.assertEqual(writer['Condition']['ArnEquals']['aws:PrincipalArn'], {'GetAtt': 'RecoveryWriterRole.Arn'})
        for action in ['s3:GetObject', 's3:GetObjectVersion', 's3:DeleteObject', 's3:DeleteObjectVersion', 's3:BypassGovernanceRetention', 's3:PutObjectRetention', 's3:PutObjectLegalHold', 's3:PutBucketPolicy', 's3:DeleteBucketPolicy', 's3:PutBucketObjectLockConfiguration', 's3:PutBucketVersioning', 's3:PutLifecycleConfiguration']:
            self.assertTrue(denied(writer, action), action)
        reader = next(s for s in statements if s['Sid'] == 'RestrictReaderEvenWithAdditionalIdentityGrants')
        for action in ['s3:PutObject', 's3:DeleteObject', 's3:DeleteObjectVersion', 's3:PutObjectRetention', 's3:BypassGovernanceRetention', 's3:PutBucketPolicy']:
            self.assertTrue(denied(reader, action), action)
        for action in ['s3:GetObject', 's3:GetObjectVersion', 's3:ListBucketVersions']:
            self.assertFalse(denied(reader, action))
        encryption = next(s for s in statements if s['Sid'] == 'RequireSSES3')
        self.assertEqual(encryption['Condition'], {'StringNotEquals': {'s3:x-amz-server-side-encryption': 'AES256'}})

    def testAbsentAutomationBreachesDailyAlarm(self):
        alarm = RESOURCES['RecoverySetAlarm']['Properties']
        self.assertEqual(alarm['MetricName'], 'RecoverySetComplete')
        self.assertEqual(alarm['Namespace'], 'Scope/Security/Recovery')
        self.assertEqual(alarm['Dimensions'], [{'Name': 'BucketName', 'Value': {'Ref': 'RecoveryBucket'}}])
        self.assertEqual(alarm['TreatMissingData'], 'breaching')
        self.assertEqual(alarm['Period'], 86400)
        self.assertEqual(alarm['EvaluationPeriods'], 2)
        self.assertEqual(alarm['DatapointsToAlarm'], 2)
        self.assertEqual(alarm['Statistic'], 'Minimum')
        self.assertEqual(alarm['ComparisonOperator'], 'LessThanThreshold')
        self.assertEqual(alarm['Threshold'], 1)
        metric = RESOURCES['RecoveryWriterRole']['Properties']['Policies'][0]['PolicyDocument']['Statement'][-1]
        self.assertEqual(metric['Condition'], {'StringEquals': {'cloudwatch:namespace': alarm['Namespace']}})


if __name__ == '__main__':
    unittest.main()
