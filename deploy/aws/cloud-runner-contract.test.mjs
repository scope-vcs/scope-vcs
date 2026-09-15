import assert from 'node:assert/strict'
import { readFileSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'

const template = readFileSync(new URL('./cloud-runner.yaml', import.meta.url), 'utf8')
const infrastructureWorkflow = readFileSync(
  new URL('../../.github/workflows/scope-aws-infrastructure.yml', import.meta.url),
  'utf8',
)

function between(source, start, end) {
  const startIndex = source.indexOf(start)
  const endIndex = source.indexOf(end, startIndex + start.length)
  assert.notEqual(startIndex, -1, `missing ${start}`)
  assert.notEqual(endIndex, -1, `missing ${end}`)
  return source.slice(startIndex, endIndex)
}

test('private registry credentials remain an optional exact secret grant', () => {
  const parameters = between(template, 'Parameters:', 'Conditions:')
  assert.match(parameters, /RegistryCredentialsSecretArn:\n\s+Type: String\n\s+Default: ""/)
  assert.match(
    parameters,
    /AllowedPattern: "\^\$\|\^arn:\(aws\|aws-us-gov\|aws-cn\):secretsmanager:/,
  )
  assert.match(
    template,
    /HasRegistryCredentialsSecret: !Not \[!Equals \[!Ref RegistryCredentialsSecretArn, ""\]\]/,
  )

  const executionRole = between(template, '  RunnerTaskExecutionRole:', '  RailwayDispatcherUser:')
  assert.match(
    executionRole,
    /- !If\n\s+- HasRegistryCredentialsSecret\n\s+- Sid: ReadPrivateRegistryCredentials[\s\S]*?Action: secretsmanager:GetSecretValue\n\s+Resource: !Ref RegistryCredentialsSecretArn\n\s+- !Ref AWS::NoValue/,
  )
  assert.doesNotMatch(
    executionRole,
    /Action: secretsmanager:GetSecretValue\n\s+Resource: ["']?\*["']?/,
  )
})

test('the Railway dispatcher can invoke only the exact broker function', () => {
  const dispatcher = between(template, '  RailwayDispatcherPolicy:', '  RunnerBudget:')
  assert.equal((dispatcher.match(/Effect: Allow/g) ?? []).length, 1)
  assert.match(dispatcher, /Action: lambda:InvokeFunction/)
  assert.match(dispatcher, /Resource: !Sub arn:\$\{AWS::Partition\}:lambda:\$\{AWS::Region\}:\$\{AWS::AccountId\}:function:scope-cloud-runner-\$\{Environment\}-dispatch-broker/)
  assert.doesNotMatch(dispatcher, /ecs:|secretsmanager:|iam:PassRole|Resource: ["']?\*/)
})

test('the registry credentials ARN is a repository variable, not a secret', () => {
  assert.doesNotMatch(infrastructureWorkflow, /secrets\.SCOPE_REGISTRY_CREDENTIALS_SECRET_ARN/)
})

const executionWorkflow = readFileSync(new URL('../../.github/workflows/scope-aws-infrastructure-execute.yml', import.meta.url), 'utf8')
const brokerScript = new URL('./apply-dispatch-broker.sh', import.meta.url).pathname
const account = '123456789012'
const executionRole = `arn:aws:iam::${account}:role/scope-infrastructure-execution`
const brokerChangeSet = `arn:aws:cloudformation:us-east-1:${account}:changeSet/broker-review/identifier`

function brokerOperation(t, command, overrides = {}, changeSet = '') {
  const directory = mkdtempSync(join(tmpdir(), 'scope-broker-plan-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  const callsPath = join(directory, 'calls.jsonl')
  writeFileSync(join(directory, 'aws'), `#!/usr/bin/env node
    const { appendFileSync } = require('node:fs');
    const args = process.argv.slice(2);
    appendFileSync(process.env.FAKE_AWS_CALLS, JSON.stringify(args) + '\\n');
    const command = args.slice(2, 4).join(' ');
    if (command === 'sts get-caller-identity') console.log('arn:aws:sts::${account}:assumed-role/infrastructure/test\\t${account}');
    else if (command === 'cloudformation describe-stacks') console.log(process.env.FAKE_STACK_STATUS || 'UPDATE_COMPLETE');
    else if (command === 'cloudformation create-change-set') console.log('${brokerChangeSet}');
    else console.log('validated');
  `, { mode: 0o755 })
  const env = { ...process.env, PATH: `${directory}:${process.env.PATH}`, FAKE_AWS_CALLS: callsPath,
    AWS_REGION: 'us-east-1', SCOPE_AWS_EXECUTION_ROLE_ARN: executionRole }
  for (const key of ['SCOPE_BROKER_CODE_BUCKET', 'SCOPE_BROKER_CODE_KEY', 'SCOPE_BROKER_CODE_VERSION', 'FAKE_STACK_STATUS']) delete env[key]
  const result = spawnSync('bash', [brokerScript, command, changeSet], {
    env: { ...env, ...overrides }, encoding: 'utf8', timeout: 10000,
  })
  assert.ifError(result.error)
  let calls = []
  try { calls = readFileSync(callsPath, 'utf8').trim().split('\n').map(JSON.parse) } catch {}
  return { ...result, calls }
}

const brokerCode = {
  SCOPE_BROKER_CODE_BUCKET: `scope-dispatch-broker-artifacts-${account}-us-east-1`,
  SCOPE_BROKER_CODE_KEY: 'reviewed/broker.zip',
  SCOPE_BROKER_CODE_VERSION: 'immutable-version.123',
}

test('protected infrastructure workflow routes an explicit broker target without secret inputs', () => {
  assert.match(infrastructureWorkflow, /options: \[runner, broker\]/)
  assert.match(infrastructureWorkflow, /target: \$\{\{ inputs.target \}\}/)
  assert.match(executionWorkflow, /environment: AWS-infrastructure/)
  assert.match(executionWorkflow, /case "\$TARGET" in/)
  assert.match(executionWorkflow, /broker\) deploy\/aws\/apply-dispatch-broker.sh "\$COMMAND" "\$CHANGE_SET_ARN"/)
  for (const field of ['code_bucket', 'code_key', 'code_version']) assert.ok(executionWorkflow.includes(`inputs.${field}`))
  assert.doesNotMatch(infrastructureWorkflow + executionWorkflow, /DispatchAuthorityToken|dispatch_authority_token|secrets\./)
})

test('broker plans bind an immutable code version and preserve existing secret and topology parameters', t => {
  const result = brokerOperation(t, 'plan', { ...brokerCode, SCOPE_DISPATCH_BROKER_TOKEN: 'must-not-leak' })
  assert.equal(result.status, 0, result.stderr)
  const create = result.calls.find(call => call.includes('create-change-set'))
  assert.ok(create)
  assert.equal(create[create.indexOf('--role-arn') + 1], executionRole)
  assert.equal(create[create.indexOf('--change-set-type') + 1], 'UPDATE')
  for (const key of ['Environment', 'ApiUrl', 'DispatchAuthorityToken', 'ClusterArn', 'SubnetIds', 'SecurityGroupId', 'ExecutionRoleArn', 'RunnerLogGroup', 'RegistryCredentialsSecretArn', 'RegistryCredentialsHost']) {
    assert.ok(create.includes(`ParameterKey=${key},UsePreviousValue=true`), key)
  }
  assert.ok(create.includes('ParameterKey=CodeVersion,ParameterValue=immutable-version.123'))
  assert.ok(result.calls.every(call => !call.includes('execute-change-set')))
  assert.doesNotMatch(JSON.stringify(result.calls) + result.stdout + result.stderr, /must-not-leak/)
})

test('broker plans reject unversioned code, another bucket, and missing bootstrap', t => {
  for (const override of [
    { SCOPE_BROKER_CODE_VERSION: '' },
    { SCOPE_BROKER_CODE_VERSION: 'null' },
    { SCOPE_BROKER_CODE_BUCKET: 'another-bucket' },
    { FAKE_STACK_STATUS: 'REVIEW_IN_PROGRESS' },
  ]) {
    const result = brokerOperation(t, 'plan', { ...brokerCode, ...override })
    assert.notEqual(result.status, 0)
    assert.ok(result.calls.every(call => !call.includes('create-change-set') && !call.includes('execute-change-set')))
  }
})

test('broker apply executes only an exact reviewed change set on the fixed existing stack', t => {
  assert.notEqual(brokerOperation(t, 'apply').status, 0)
  assert.notEqual(brokerOperation(t, 'apply', {}, 'mutable-plan-name').status, 0)
  assert.notEqual(brokerOperation(t, 'apply', {}, brokerChangeSet.replace(account, '999999999999')).status, 0)
  const withCode = brokerOperation(t, 'apply', brokerCode, brokerChangeSet)
  assert.notEqual(withCode.status, 0)
  assert.ok(withCode.calls.every(call => !call.includes('execute-change-set')))
  const result = brokerOperation(t, 'apply', {}, brokerChangeSet)
  assert.equal(result.status, 0, result.stderr)
  const execute = result.calls.find(call => call.includes('execute-change-set'))
  assert.equal(execute[execute.indexOf('--stack-name') + 1], 'scope-dispatch-broker-production')
  assert.equal(execute[execute.indexOf('--change-set-name') + 1], brokerChangeSet)
  assert.ok(result.calls.every(call => !call.includes('create-change-set')))
  assert.ok(result.calls.filter(call => call.includes('describe-change-set')).every(call => call.includes('Changes[].ResourceChange.[Action,LogicalResourceId,ResourceType,Replacement]')))
})
