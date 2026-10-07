export type Preset = { id: string; name: string; api_format: 'openai' | 'anthropic'; base_url: string; local?: boolean };

export const PRESETS: Preset[] = [
	{ id: 'openai', name: 'OpenAI', api_format: 'openai', base_url: 'https://api.openai.com/v1' },
	{ id: 'anthropic', name: 'Anthropic', api_format: 'anthropic', base_url: 'https://api.anthropic.com/v1' },
	{ id: 'openrouter', name: 'OpenRouter', api_format: 'openai', base_url: 'https://openrouter.ai/api/v1' },
	{ id: 'groq', name: 'Groq', api_format: 'openai', base_url: 'https://api.groq.com/openai/v1' },
	{ id: 'together', name: 'Together', api_format: 'openai', base_url: 'https://api.together.xyz/v1' },
	{ id: 'mistral', name: 'Mistral', api_format: 'openai', base_url: 'https://api.mistral.ai/v1' },
	{ id: 'deepseek', name: 'DeepSeek', api_format: 'openai', base_url: 'https://api.deepseek.com/v1' },
	{ id: 'ollama', name: 'Ollama (local)', api_format: 'openai', base_url: 'http://localhost:11434/v1', local: true },
	{ id: 'vllm', name: 'vLLM / LM Studio', api_format: 'openai', base_url: 'http://localhost:8000/v1', local: true },
	{ id: 'custom', name: 'Custom', api_format: 'openai', base_url: '' }
];
