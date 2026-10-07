import { get } from './api';

export type User = { id: string; email: string; name: string; role: 'admin' | 'user'; must_reset_password: boolean };
export type Instance = { name: string; logo_url: string | null; needs_setup: boolean; open_signup: boolean; email_enabled: boolean };
export type Me = { user: User; default_model_id: string | null; can_use_api_keys: boolean; show_metrics: boolean };

export const session = $state<{ instance: Instance | null; me: Me | null; loaded: boolean }>({
	instance: null,
	me: null,
	loaded: false
});

export async function loadSession() {
	session.instance = await get<Instance>('/api/instance');
	session.me = session.instance.needs_setup ? null : await get<Me>('/api/auth/me').catch(() => null);
	session.loaded = true;
}
