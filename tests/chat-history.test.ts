import { describe, expect, test } from 'bun:test';
import { activeConversation, emptyChatHistory, modelConversation, titleFromPrompt } from '../src/lib/chat-history';

describe('histórico do chat', () => {
  test('contexto distingue alterações propostas das aplicadas', () => {
    const entry = { role: 'assistant' as const, content: 'Alteração pronta.', timestamp: '09:00', edits: [
      { path: 'Plano.md', old_text: '- [ ] Estudar', new_text: '- [x] Estudar', appliedPath: undefined as string | undefined },
    ] };
    expect(modelConversation([entry])[0].content).toContain('"applied":false');
    entry.edits[0].appliedPath = 'Plano.md';
    expect(modelConversation([entry])[0].content).toContain('"applied":true');
  });
  test('nova conversa e títulos são independentes de mensagens antigas', () => {
    const history = emptyChatHistory();
    expect(activeConversation(history)).toBeNull();
    history.conversations.push({ id: 'one', title: titleFromPrompt('Aprender Python\ncom exercícios'),
      createdAt: 1, updatedAt: 2, messages: [{ role: 'user', content: 'Olá', timestamp: '09:00' }] });
    history.activeConversationId = 'one';
    expect(activeConversation(history)?.title).toBe('Aprender Python');
    history.activeConversationId = null;
    expect(activeConversation(history)).toBeNull();
    expect(history.conversations[0].messages).toHaveLength(1);
  });

  test('contexto do modelo exclui erros e limita documentos extensos', () => {
    const history = emptyChatHistory();
    const messages = [
      { role: 'user' as const, content: 'Crie um plano', timestamp: '09:00' },
      { role: 'assistant' as const, content: 'Pronto', timestamp: '09:01', drafts: [
        { path: 'Plano.md', content: 'x'.repeat(10000) },
      ] },
      { role: 'assistant' as const, content: 'Erro temporário', timestamp: '09:02', isError: true },
    ];
    history.conversations.push({ id: 'one', title: 'Plano', createdAt: 1, updatedAt: 2, messages });
    expect(modelConversation(messages)).toHaveLength(2);
    expect(modelConversation(messages)[1].content).toContain('Plano.md');
    expect(modelConversation(messages)[1].content.length).toBeLessThan(2000);
  });
});
