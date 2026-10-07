export class ApiError extends Error {
	constructor(
		public status: number,
		public code: string,
		message: string
	) {
		super(message);
	}
}

export async function api<T = any>(method: string, path: string, body?: unknown): Promise<T> {
	const res = await fetch(path, {
		method,
		headers: {
			'x-braid-csrf': '1',
			...(body !== undefined ? { 'content-type': 'application/json' } : {})
		},
		body: body !== undefined ? JSON.stringify(body) : undefined
	});
	const data = res.headers.get('content-type')?.includes('json') ? await res.json() : null;
	if (!res.ok) {
		throw new ApiError(res.status, data?.error?.code ?? 'error', data?.error?.message ?? res.statusText);
	}
	return data as T;
}

export const get = <T = any>(path: string) => api<T>('GET', path);
export const post = <T = any>(path: string, body: unknown = {}) => api<T>('POST', path, body);
export const put = <T = any>(path: string, body: unknown = {}) => api<T>('PUT', path, body);
export const patch = <T = any>(path: string, body: unknown = {}) => api<T>('PATCH', path, body);
export const del = <T = any>(path: string) => api<T>('DELETE', path);

export const errorMessage = (e: unknown) => (e instanceof Error ? e.message : String(e));
