/** An error the merchant or the wallet answered with, or a request that failed. */
export class ZolarError extends Error {
  constructor(
    message: string,
    readonly status: number | null,
  ) {
    super(message);
    this.name = 'ZolarError';
  }
}

export type Fetch = typeof fetch;

export async function request<T>(
  fetchImpl: Fetch,
  url: string,
  init: RequestInit & { timeoutMs?: number } = {},
): Promise<T> {
  const { timeoutMs = 30_000, ...rest } = init;
  let res: Response;
  try {
    res = await fetchImpl(url, { ...rest, signal: rest.signal ?? AbortSignal.timeout(timeoutMs) });
  } catch (e) {
    throw new ZolarError(`${url}: ${(e as Error).message}`, null);
  }
  const body = (await res.json().catch(() => null)) as (T & { error?: string }) | null;
  if (!res.ok) {
    throw new ZolarError(body?.error ?? `${url}: HTTP ${res.status}`, res.status);
  }
  if (body === null) throw new ZolarError(`${url}: the answer is not JSON`, res.status);
  return body;
}

export function base(url: string): string {
  return url.replace(/\/+$/, '');
}
