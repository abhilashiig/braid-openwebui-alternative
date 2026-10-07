export type Theme = 'system' | 'light' | 'dark';

export function currentTheme(): Theme {
	try {
		return (localStorage.getItem('theme') as Theme) || 'system';
	} catch {
		return 'system';
	}
}

export function applyTheme(theme: Theme = currentTheme()) {
	try {
		localStorage.setItem('theme', theme);
	} catch {}
	const dark = theme === 'dark' || (theme === 'system' && matchMedia('(prefers-color-scheme: dark)').matches);
	document.documentElement.classList.toggle('dark', dark);
}
