import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { prepareCutover, serviceRoles } from './prepare-cutover.mjs';
import { localClusterSkip, pgBin } from './test-cluster.mjs';

const manifest = JSON.parse(readFileSync(new URL('../../.github/deployment-services.json',import.meta.url)));
const fixtureUrl = new URL('postgresql://postgres.railway.internal:5432/scope?sslmode=prefer');
fixtureUrl.username = 'postgres';
fixtureUrl.password = 'synthetic-test-only-password';
const url = fixtureUrl.toString();

test('cutover bundles keep credentials private and target only the selected fixed services',t=>{
  const root=mkdtempSync(join(tmpdir(),'scope-cutover-prepare-'));
  t.after(()=>rmSync(root,{recursive:true,force:true}));
  const directory=join(root,'bundle');
  const summary=prepareCutover(manifest,'staging',url,directory);
  assert.deepEqual(summary,{environment:'staging',environmentId:manifest.environments.staging.environmentId,roles:6});
  assert.equal(statSync(directory).mode & 0o777,0o700);
  assert(readdirSync(directory).every(file=>(statSync(join(directory,file)).mode & 0o777)===0o600));
  const script=readFileSync(join(directory,'bootstrap.sh'),'utf8');
  assert.equal((script.match(/^BEGIN;/gm)||[]).length,1);
  assert.equal((script.match(/^COMMIT;/gm)||[]).length,1);
  assert(!script.includes('psql postgres'));
  const passwords=[];
  for(const [component,role] of Object.entries(serviceRoles)){
    const {input}=JSON.parse(readFileSync(join(directory,`${component}.variables.json`)));
    assert.equal(input.environmentId,manifest.environments.staging.environmentId);
    assert.equal(input.projectId,manifest.railway.projectId);
    assert.equal(input.serviceId,component==='maintenance'?manifest.railway.maintenanceServiceId:manifest.services[component].id);
    assert.equal(input.skipDeploys,true); assert.equal(input.replace,false);
    assert.deepEqual(Object.keys(input.variables),['DATABASE_URL']);
    const connection=new URL(input.variables.DATABASE_URL);
    assert.equal(connection.username,role);
    assert.equal(connection.hostname,'postgres.railway.internal');
    assert.equal(connection.searchParams.get('sslmode'),'require');
    assert.match(connection.password,/^[A-Za-z0-9_-]{43}$/);
    assert(!script.includes(connection.password),'SQL contains only SCRAM verifiers for new passwords');
    passwords.push(connection.password);
    assert.match(readFileSync(join(directory,`${component}.verify.sh`),'utf8'),/exec psql -X -q -v ON_ERROR_STOP=1 <</);
  }
  assert.equal(new Set(passwords).size,6);
  assert(!JSON.stringify(summary).includes('secret'));
  assert.throws(()=>prepareCutover(manifest,'staging',url,directory),/EEXIST/);
  assert.equal(readdirSync(directory).length,15,'Refused overwrites preserve the existing recovery bundle');
  const publicFixtureUrl = new URL(url);
  publicFixtureUrl.hostname = 'public.example';
  for(const bad of [publicFixtureUrl.toString(),url+'&options=bad',url+'#fragment']){
    assert.throws(()=>prepareCutover(manifest,'production',bad,join(root,'bad')),/private Railway/);
  }
  assert.throws(()=>prepareCutover(manifest,'other',url,join(root,'bad')),/staging or production/);
});

test('generated SCRAM verifiers authenticate each new password and reject a wrong password',{skip:localClusterSkip},t=>{
  const bin=pgBin;
  const root=mkdtempSync(join(tmpdir(),'scope-cutover-password-'));
  const data=join(root,'data'); let started=false;
  t.after(()=>{
    if(started) spawnSync(join(bin,'pg_ctl'),['-D',data,'-m','immediate','-w','stop'],{stdio:'pipe'});
    rmSync(root,{recursive:true,force:true});
  });
  execFileSync(join(bin,'initdb'),['-D',data,'-U','postgres','--auth=trust','--no-locale'],{stdio:'pipe'});
  execFileSync(join(bin,'pg_ctl'),['-D',data,'-l',join(root,'log'),'-o',`-k ${root} -c listen_addresses=''`,'-w','start'],{stdio:'pipe'});
  started=true;
  const directory=join(root,'bundle'); prepareCutover(manifest,'staging',url,directory);
  const bootstrap=readFileSync(join(directory,'bootstrap.sh'),'utf8');
  const credentials=Object.entries(serviceRoles).map(([component,role])=>({role,password:new URL(JSON.parse(readFileSync(join(directory,`${component}.variables.json`))).input.variables.DATABASE_URL).password}));
  const sql=credentials.map(({role})=>`CREATE ROLE ${role} LOGIN;\n${bootstrap.match(new RegExp(`ALTER ROLE ${role} PASSWORD '[^']+';`))[0]}`).join('\n');
  execFileSync(join(bin,'psql'),['-X','-q','-h',root,'-U','postgres','-d','postgres','-v','ON_ERROR_STOP=1'],{input:sql,stdio:['pipe','pipe','pipe']});
  writeFileSync(join(data,'pg_hba.conf'),'local all all scram-sha-256\n');
  execFileSync(join(bin,'pg_ctl'),['-D',data,'reload'],{stdio:'pipe'});
  for(const {role,password} of credentials){
    const result=spawnSync(join(bin,'psql'),['-X','-qAt','-h',root,'-U',role,'-d','postgres','-c','SELECT current_user'],{env:{...process.env,PGPASSWORD:password},encoding:'utf8'});
    assert.equal(result.status,0,result.stderr); assert.equal(result.stdout.trim(),role);
  }
  const rejected=spawnSync(join(bin,'psql'),['-X','-qAt','-h',root,'-U','scope_api','-d','postgres','-c','SELECT current_user'],{env:{...process.env,PGPASSWORD:'incorrect'},encoding:'utf8'});
  assert.notEqual(rejected.status,0); assert.match(rejected.stderr,/password authentication failed/);
});


test('cutover requires encrypted connections and preserves explicit certificate verification',t=>{
  const root=mkdtempSync(join(tmpdir(),'scope-cutover-tls-'));
  t.after(()=>rmSync(root,{recursive:true,force:true}));
  for(const mode of ['', 'disable', 'allow', 'prefer', 'require', 'verify-ca', 'verify-full']){
    const directory=join(root,mode || 'default');
    const input=new URL(url); input.search='';
    if(mode) input.searchParams.set('sslmode',mode);
    prepareCutover(manifest,'staging',input.toString(),directory);
    const expected=['verify-ca','verify-full'].includes(mode)?mode:'require';
    assert.match(readFileSync(join(directory,'bootstrap.sh'),'utf8'),new RegExp(`export PGSSLMODE='${expected}'`));
    assert.equal(readFileSync(join(directory,'maintenance.connection'),'utf8').trim().split('\n').at(-1),expected);
    for(const component of Object.keys(serviceRoles)){
      const payload=JSON.parse(readFileSync(join(directory,`${component}.variables.json`)));
      assert.equal(new URL(payload.input.variables.DATABASE_URL).searchParams.get('sslmode'),expected);
      assert.match(readFileSync(join(directory,`${component}.verify.sh`),'utf8'),new RegExp(`export PGSSLMODE='${expected}'`));
    }
  }
  assert.throws(()=>prepareCutover(manifest,'staging',url.replace('prefer','invalid'),join(root,'invalid')),/Unsupported PostgreSQL TLS mode/);
});
