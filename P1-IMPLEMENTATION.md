# Implementação dos itens P1

Escopo: B01–B08 de BACKLOG.md, solicitado em 07/10/2026. Este arquivo acompanha trabalho e evidências; um item só deve ser marcado concluído depois de validar seus critérios de aceite.

- [ ] B01 — exclusões duráveis, peers offline, confirmação, conflito excluir/editar, pastas.
- [ ] B02 — identidade e histórico em renomeações, referências e mapa, convergência offline.
- [ ] B03 — gravação atômica, backups válidos, recuperação coordenada e aviso ao usuário.
- [ ] B04 — lixeira e versões persistentes, retenção, comparação e restauração colaborativa.
- [ ] B05 — operações de vínculos com mesclagem e remoção durável.
- [ ] B06 — benchmarks reproduzíveis de RAM/CPU/latência e relatórios por plataforma.
- [ ] B07 — CI de PR/push com Rust/frontend e regressões de integração/interface.
- [x] B08 — armazenamento de credenciais do SO, migração e tratamento de indisponibilidade.

## Trabalho em andamento

- Base de gravação atômica com backups e recuperação, usando substituição atômica do tempfile em vez de apagar o arquivo de destino.
- B03: intenção durável por nota coordena Markdown e CRDT; recuperação conclui gravações interrompidas e preserva edições externas em uma cópia para revisão. Atualizações remotas são validadas em um documento candidato antes de substituir o cache.
- B08: referências imutáveis no armazenamento de credenciais do SO; migração lê cada segredo de volta antes de limpar o JSON e seu backup. Configuração indisponível não substitui a identidade P2P; há aviso e ação de tentar novamente. Testes cobrem migração, indisponibilidade, verificação malsucedida, recuperação de referências antigas e exportação sem segredos.
- B07: CI de PR/push para Windows, Linux e macOS, com Rust, frontend, testes nativos de credenciais e Playwright. O adapter de IPC do navegador é isolado; os testes Rust verificam persistência/protocolo reais.
- B05: histórico imutável de adições/remoções em `.lownotes/link-operations.json`, com remoção das adições observadas e migração idempotente de listas antigas. Sincronização usa a união das operações, independentemente de timestamps. Origens ficam preservadas; uma adição realmente concorrente sobrevive a uma remoção que ainda não a viu. O protocolo `/4` transmite JSON sem expansão para arrays numéricos e mantém fallback `/3` e `/2`.
- B07: o workflow de release agora exige as regressões Rust e de interface antes da publicação. O cenário de nota mais recente usa um timestamp explícito; duas gravações rápidas no Windows podem cair no mesmo milissegundo.
- Ícones novos e o backlog previamente criados permanecem no workspace; não são usados como prova de conclusão dos P1.

## Evidências locais (Windows)

- `bun test`: 90 testes passaram.
- `bun run check` e `bun run check:e2e`: sem erros.
- `bun run build`: compilação concluída.
- `bun run test:e2e -- --workers 2`: 11 cenários passaram, incluindo tarefas/desfazer/refazer com texto remoto, busca nos três modos, links, todas as paletas/modos, recuperação e credenciais indisponíveis.
- `cargo test --locked --lib --manifest-path src-tauri/Cargo.toml`: 93 testes passaram; 4 fixtures/testes condicionais ficaram ignorados na execução geral.
- `credentials::tests::native_store_round_trip -- --exact --ignored`: passou no Credential Manager do Windows; usou e removeu uma entrada sintética.
- Recuperação após `process::exit(86)` comprovada em três etapas: intenção durável, estado CRDT gravado e Markdown gravado.
- B05: teste com três endpoints Iroh reais, adições e remoção offline, timestamps invertidos e replay de snapshots/listas antigos. Inclui concorrência local de 20 ações, reconstrução de uma projeção antiga, validação de mensagens, conservação de origens, um histórico acima de 10 MB e fallback de imagens para `/3`.

## Evidências remotas e pendências

- CI `37671886130` sobre `d2e3177`: Linux e macOS concluíram Rust, credenciais nativas e os 11 cenários de interface. Windows teve uma falha de timestamp no cenário de atualização incremental, corrigida e validada localmente; os testes nativos do Windows já haviam passado em execução local com uma entrada sintética.
- B08 está comprovado por testes de migração/indisponibilidade/exportação e pelos backends nativos dos três sistemas. Nenhuma configuração real do usuário foi usada pelos testes.
- As alterações B05 e as novas etapas de release ainda precisam passar no novo CI remoto; B05 continua sem marca de conclusão até essa conferência.
- B01, B02, B04 e B06 continuam pendentes. B03 e B07 têm a base implementada e aguardam a cobertura completa das operações restantes, incluindo renomeação, exclusão e restauração. A gravação nativa direta de Markdown também deve preservar o histórico CRDT durante B02.
