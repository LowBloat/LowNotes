import type { ChatConversation, ChatHistory, ChatMessage, StoredChatEntry } from './types';

export function emptyChatHistory(): ChatHistory {
  return { version: 1, activeConversationId: null, conversations: [], memory: '' };
}

export function titleFromPrompt(prompt: string): string {
  const firstLine = prompt.trim().split('\n', 1)[0].replace(/\s+/g, ' ');
  return firstLine.length > 54 ? `${firstLine.slice(0, 53)}…` : firstLine || 'Nova conversa';
}

export function activeConversation(history: ChatHistory): ChatConversation | null {
  return history.conversations.find((chat) => chat.id === history.activeConversationId) ?? null;
}

/** Send only a bounded, relevant context to the model; the full history stays available in the UI. */
export function modelConversation(messages: StoredChatEntry[]): ChatMessage[] {
  return messages.filter((message) => !message.isError).slice(-8).map((message) => {
    const drafts = message.drafts?.length
      ? `\nDocuments from this response: ${JSON.stringify(message.drafts.map((draft) => ({
          path: draft.savedPath || draft.path,
          saved: !!draft.savedPath,
          preview: draft.savedPath ? undefined : draft.content.slice(0, 1200),
        })))}`
      : '';
    const edits = message.edits?.length ? `\nNote edits from this response: ${JSON.stringify(message.edits.map((edit) => ({
      path: edit.path, applied: !!edit.appliedPath,
      old_text: edit.old_text.slice(0, 600), new_text: edit.new_text.slice(0, 600),
    })))}` : '';
    return { role: message.role, content: `${message.content.slice(0, 6000)}${drafts}${edits}` };
  });
}
