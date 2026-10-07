export function timeAgo(iso: string | null | undefined): string {
	if (!iso) return 'never';
	const s = (Date.now() - new Date(iso).getTime()) / 1000;
	if (s < 60) return 'just now';
	if (s < 3600) return `${Math.floor(s / 60)} min ago`;
	if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
	if (s < 86400 * 30) return `${Math.floor(s / 86400)} d ago`;
	return new Date(iso).toLocaleDateString();
}

export const fmtDate = (iso: string) => new Date(iso).toLocaleString();

export function fmtNum(n: number | null | undefined): string {
	if (n == null) return '–';
	if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
	if (n >= 1e4) return `${(n / 1e3).toFixed(1)}k`;
	return n.toLocaleString();
}

export const fmtCost = (c: number | null | undefined) => (c == null ? '–' : c < 0.01 ? `$${c.toFixed(4)}` : `$${c.toFixed(2)}`);
