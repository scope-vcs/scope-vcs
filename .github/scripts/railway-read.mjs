#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const ATTEMPTS = 3;
const FAILURE = "Railway read failed after 3 attempts";

function singleQuery(query) {
  if (!/^query\s/.test(query)) return false;
  // Ignore quoted values and comments while checking document boundaries.
  const document = query.replace(/"""(?:\\[\s\S]|[^\\])*?"""|"(?:\\.|[^"\\])*"|#[^\r\n]*/g, '""');
  if (/\b(?:mutation|subscription)\b/.test(document)) return false;
  const delimiters = [];
  for (let index = 0; index < document.length; index += 1) {
    const character = document[index];
    if ("({[".includes(character)) delimiters.push(character);
    if (")}]".includes(character)) {
      if (delimiters.pop() !== { ")": "(", "}": "{", "]": "[" }[character]) return false;
      if (character === "}" && delimiters.length === 0) {
        return document.slice(index + 1).trim() === "";
      }
    }
  }
  return false;
}

function validateRead(args, input) {
  if (!Array.isArray(args) || !args.every((argument) => typeof argument === "string")) {
    throw new Error("Unsupported Railway read command");
  }
  const api = args[0] === "api";
  const stdinVariables = api && args.length === 4 && args[2] === "--variables" && args[3] === "@-";
  const allowed = api
    ? (args.length === 2 || stdinVariables) && singleQuery(args[1])
    : args[0] === "status" || (["service", "deployment", "variable"].includes(args[0]) && args[1] === "list");
  if (!allowed || (stdinVariables ? typeof input !== "string" : input !== undefined)) {
    throw new Error("Unsupported Railway read command");
  }
}

function pauseSync(milliseconds) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds);
}

export function readRailway(args, {
  input,
  execute = execFileSync,
  pause = pauseSync,
  report = (message) => console.error(message),
} = {}) {
  validateRead(args, input);
  for (let attempt = 1; attempt <= ATTEMPTS; attempt += 1) {
    try {
      const result = JSON.parse(execute("railway", args, {
        input,
        encoding: "utf8",
        stdio: ["pipe", "pipe", "pipe"],
        timeout: 30_000,
        killSignal: "SIGKILL",
      }));
      if (args[0] === "api" && result?.errors?.length) throw new Error(FAILURE);
      return result;
    } catch {
      // Failed commands and partial responses may contain secret variables.
      if (attempt === ATTEMPTS) throw new Error(FAILURE);
      report(`Railway read failed; retrying (${attempt}/${ATTEMPTS})`);
      pause(2_000);
    }
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const args = process.argv.slice(2);
    const input = args[0] === "api" && args[2] === "--variables" && args[3] === "@-"
      ? readFileSync(0, "utf8")
      : undefined;
    process.stdout.write(`${JSON.stringify(readRailway(args, { input }))}\n`);
  } catch {
    console.error(FAILURE);
    process.exitCode = 1;
  }
}
