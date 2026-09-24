import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';

const minimum = JSON.parse(readFileSync(new URL('./tool-versions.json', import.meta.url))).git.version;
const actual = execFileSync('git', ['--version'], { encoding: 'utf8' }).trim();
const expectedParts = minimum.split('.').map(Number);
const actualParts = /^git version (\d+)\.(\d+)\.(\d+)/.exec(actual)?.slice(1).map(Number);
const firstDifference = actualParts?.findIndex((part, index) => part !== expectedParts[index]);

if (!actualParts || (firstDifference !== -1
    && actualParts[firstDifference] < expectedParts[firstDifference])) {
  console.error(`Git ${minimum} or newer is required; found ${actual}.`);
  process.exitCode = 1;
} else {
  console.log(actual);
}
