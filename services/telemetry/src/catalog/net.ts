/**
 * Outbound calls for catalog enrichment: a fetch and a sleep the tests replace, and a budget of
 * subrequests so one run stays under the Workers limit (50 on the free plan) and never times out.
 */

export type Net = {
  readonly fetch: (input: string, init?: RequestInit) => Promise<Response>;
  readonly sleep: (ms: number) => Promise<void>;
};

export const liveNet: Net = {
  fetch: (input, init) => fetch(input, init),
  sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
};

/** Subrequests left in this run. Every GameBanana, Steam and TypeSafe call takes one. */
export class Budget {
  constructor(private left: number) {}

  take(): boolean {
    if (this.left <= 0) return false;
    this.left -= 1;
    return true;
  }

  get remaining(): number {
    return this.left;
  }
}

/** Thrown when the budget is spent: the run stops cleanly and resumes next time. */
export class BudgetSpent extends Error {
  override readonly name = "BudgetSpent";
}

/** A remote said no (status) or did not answer. The run stops on the first one. */
export class RemoteError extends Error {
  override readonly name = "RemoteError";
  constructor(
    readonly host: string,
    readonly status: number,
  ) {
    super(`${host} answered ${status}`);
  }
}
