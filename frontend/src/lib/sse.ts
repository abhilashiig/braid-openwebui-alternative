import { ApiError } from './api.ts';

/** POSTs JSON and dispatches server-sent events as they arrive. Resolves when the stream ends. */
export async function postSSE(url: string, body: unknown, signal: AbortSignal, onEvent: (event: string, data: any) => void) {
	const res = await fetch(url, {
		method: 'POST',
		headers: { 'content-type': 'application/json', 'x-braid-csrf': '1', accept: 'text/event-stream' },
		body: JSON.stringify(body),
		signal
	});
	if (!res.ok || !res.body) {
		const data = await res.json().catch(() => null);
		throw new ApiError(res.status, data?.error?.code ?? 'error', data?.error?.message ?? res.statusText);
	}
	const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
	let buf = '';
	for (;;) {
		const { value, done } = await reader.read();
		if (done) break;
		buf += value.replace(/\r\n/g, '\n');
		let idx;
		while ((idx = buf.indexOf('\n\n')) >= 0) {
			const raw = buf.slice(0, idx);
			buf = buf.slice(idx + 2);
			let event = 'message';
			const data: string[] = [];
			for (const line of raw.split('\n')) {
				if (line.startsWith('event:')) event = line.slice(6).trim();
				else if (line.startsWith('data:')) data.push(line.slice(5).replace(/^ /, ''));
			}
			if (data.length) onEvent(event, JSON.parse(data.join('\n')));
		}
	}
}
