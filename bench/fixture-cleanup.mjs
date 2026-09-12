import { rm } from 'node:fs/promises';

// Register allocations before the next fallible setup step, including retry attempts.
export class FixtureCleanup {
  #repositories = [];
  #directories = [];
  #deleteRepository;

  constructor(deleteRepository) { this.#deleteRepository = deleteRepository; }

  repository(fixture) {
    this.#repositories.push(fixture);
    return fixture;
  }

  directory(path, kind) {
    this.#directories.push({ path, kind });
    return path;
  }

  async run() {
    const result = {
      attemptedRepositories: this.#repositories.length,
      attemptedClients: this.#directories.filter(({ kind }) => kind === 'client').length,
      attemptedDirectories: this.#directories.length,
      failed: [],
    };
    await Promise.all([
      ...this.#repositories.map(async (fixture) => {
        try { await this.#deleteRepository(fixture); }
        catch (error) { result.failed.push({ repo: `${fixture.owner}/${fixture.repo}`, error: String(error.message || error) }); }
      }),
      ...this.#directories.map(async ({ path }) => {
        try { await rm(path, { recursive: true, force: true }); }
        catch (error) { result.failed.push({ path, error: String(error.message || error) }); }
      }),
    ]);
    return result;
  }
}
