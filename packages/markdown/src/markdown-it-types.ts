// markdown-it 15 exports its instance and token types by name; the default
// export is the callable factory. Re-exported here so plugins share one import.
export type { MarkdownIt, Renderer, RendererRule, StateCore, Token } from 'markdown-it';
