// Environment variable readers shared by the bench entrypoints.

export function required(name, env = process.env) {
  const value = env[name]?.trim();
  if (!value) throw new Error(`${name} is required`);
  return value;
}

export function nonEmpty(name, fallback, env = process.env) {
  return env[name]?.trim() || fallback;
}

export function positiveInteger(name, fallback, env = process.env) {
  const value = Number.parseInt(env[name] || String(fallback), 10);
  if (!Number.isInteger(value) || value < 1) throw new Error(`${name} must be a positive integer`);
  return value;
}

export function positiveNumber(name, fallback, env = process.env) {
  const value = Number(env[name] || String(fallback));
  if (!Number.isFinite(value) || value <= 0) throw new Error(`${name} must be positive`);
  return value;
}

export function nonNegativeNumber(name, fallback, env = process.env) {
  const value = Number(env[name] || String(fallback));
  if (!Number.isFinite(value) || value < 0) throw new Error(`${name} must be non-negative`);
  return value;
}

export function boundedNumber(name, fallback, minimum, maximum, env = process.env) {
  const value = Number(env[name] || String(fallback));
  if (!Number.isFinite(value) || value < minimum || value > maximum) {
    throw new Error(`${name} must be between ${minimum} and ${maximum}`);
  }
  return value;
}

export function enumValue(name, fallback, allowed, env = process.env) {
  const value = nonEmpty(name, fallback, env);
  if (!allowed.has(value)) throw new Error(`${name} must be one of ${[...allowed].join(', ')}`);
  return value;
}
