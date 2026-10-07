import { get } from './api.ts';

export type ModelInfo = {
	id: string;
	name: string;
	display_name: string;
	description: string;
	provider_name: string;
	supports_vision: boolean;
	supports_tools: boolean;
	supports_reasoning: boolean;
	forced_skills: string[];
};

export type ChatSummary = { id: string; title: string; updated_at: string };

export const store = $state<{ chats: ChatSummary[]; q: string; models: ModelInfo[]; skills: string[]; modelsLoaded: boolean }>({
	chats: [],
	q: '',
	models: [],
	skills: [],
	modelsLoaded: false
});

export async function refreshChats() {
	store.chats = await get(`/api/chats${store.q ? `?q=${encodeURIComponent(store.q)}` : ''}`);
}

export async function loadModels() {
	const r = await get('/api/models');
	store.models = r.models;
	store.skills = r.skills;
	store.modelsLoaded = true;
}

/** A message typed on the new-chat page, sent once the chat page for the created chat mounts. */
export const pendingSend: { chatId: string | null; content: string; attachments: any[]; modelId: string } = {
	chatId: null,
	content: '',
	attachments: [],
	modelId: ''
};
