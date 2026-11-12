#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { describeProcessFailure, describeRailwayErrors, parseRailwayResponse } from "./railway-retry.mjs";

const ATTEMPTS = 3;
const READ_TIMEOUT_MS = 30_000;
const FAILURE = "Railway read failed after 3 attempts";

function singleQuery(query) {
  if (!/^query\s/.test(query)) return false;
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

function readOperation(args) {
  if (args[0] === "api") return args[1].match(/^query\s+(\w+)/)?.[1] ?? "query";
  return args[0] === "status" ? "status" : `${args[0]} ${args[1]}`;
}

function sentVariables(input) {
  try {
    return JSON.parse(input);
  } catch {
    return input;
  }
}

function readOnce(args, input, variables, execute) {
  const api = args[0] === "api";
  let output;
  try {
    output = execute("railway", args, {
      input,
      encoding: "utf8",
      stdio: ["pipe", "pipe", "pipe"],
      timeout: READ_TIMEOUT_MS,
      killSignal: "SIGKILL",
    });
  } catch (error) {
    const errors = api ? parseRailwayResponse(error?.stdout)?.errors : undefined;
    return {
      detail: errors?.length
        ? describeRailwayErrors(errors, variables)
        : describeProcessFailure(error, READ_TIMEOUT_MS),
    };
  }
  const response = parseRailwayResponse(output);
  if (!response) return { detail: "response was not JSON" };
  if (api && response.errors?.length) return { detail: describeRailwayErrors(response.errors, variables) };
  return { response };
}

export function readRailway(args, {
  input,
  execute = execFileSync,
  pause = pauseSync,
  report = (message) => console.error(message),
} = {}) {
  validateRead(args, input);
  const operation = readOperation(args);
  const variables = sentVariables(input);
  for (let attempt = 1; ; attempt += 1) {
    const read = readOnce(args, input, variables, execute);
    if (!("detail" in read)) return read.response;
    if (attempt === ATTEMPTS) throw new Error(`${FAILURE} (${operation}: ${read.detail})`);
    report(`Railway read failed (${operation}: ${read.detail}); retrying (${attempt}/${ATTEMPTS})`);
    pause(2_000);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const args = process.argv.slice(2);
    const retryDelay = Number(process.env.SCOPE_RAILWAY_READ_RETRY_DELAY_MS ?? 2_000);
    if (!Number.isInteger(retryDelay) || retryDelay < 0 || retryDelay > 2_000) {
      throw new Error('Invalid Railway retry delay');
    }
    const input = args[0] === "api" && args[2] === "--variables" && args[3] === "@-"
      ? readFileSync(0, "utf8")
      : undefined;
    process.stdout.write(`${JSON.stringify(readRailway(args, { input, pause: () => pauseSync(retryDelay) }))}\n`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
