# Implementação dos itens P1

Escopo: B01–B08 de BACKLOG.md, solicitado em 07/10/2026. Este arquivo acompanha trabalho e evidências; um item só deve ser marcado concluído depois de validar seus critérios de aceite.

- [ ] B01 — exclusões duráveis, peers offline, confirmação, conflito excluir/editar, pastas.
- [ ] B02 — identidade e histórico em renomeações, referências e mapa, convergência offline.
- [ ] B03 — gravação atômica, backups válidos, recuperação coordenada e aviso ao usuário.
- [ ] B04 — lixeira e versões persistentes, retenção, comparação e restauração colaborativa.
- [ ] B05 — operações de vínculos com mesclagem e remoção durável.
- [ ] B06 — benchmarks reproduzíveis de RAM/CPU/latência e relatórios por plataforma.
- [ ] B07 — CI de PR/push com Rust/frontend e regressões de integração/interface.
- [ ] B08 — armazenamento de credenciais do SO, migração e tratamento de indisponibilidade.

## Trabalho em andamento

- Base de gravação atômica com backups e recuperação, usando substituição atômica do tempfile em vez de apagar o arquivo de destino.
- B03: intenção durável por nota coordena Markdown e CRDT; recuperação conclui gravações interrompidas e preserva edições externas em uma cópia para revisão. Atualizações remotas são validadas em um documento candidato antes de substituir o cache.
- B08: referências imutáveis no armazenamento de credenciais do SO; migração lê cada segredo de volta antes de limpar o JSON e seu backup. Configuração indisponível não substitui a identidade P2P; há aviso e ação de tentar novamente. Testes cobrem migração, indisponibilidade, verificação malsucedida, recuperação de referências antigas e exportação sem segredos.
- B07: CI de PR/push para Windows, Linux e macOS, com Rust, frontend, testes nativos de credenciais e Playwright. O adapter de IPC do navegador é isolado; os testes Rust verificam persistência/protocolo reais.
- Ícones novos e o backlog previamente criados permanecem no workspace; não são usados como prova de conclusão dos P1.

## Evidências locais (Windows)

- `bun test`: 90 testes passaram.
- `bun run check` e `bun run check:e2e`: sem erros.
- `bun run build`: compilação concluída.
- `bun run test:e2e -- --workers 2`: 11 cenários passaram, incluindo tarefas/desfazer/refazer com texto remoto, busca nos três modos, links, todas as paletas/modos, recuperação e credenciais indisponíveis.
- `cargo test --locked --lib --manifest-path src-tauri/Cargo.toml`: 83 testes passaram antes da última regressão de timestamps; 4 fixtures/testes condicionais ficaram ignorados na execução geral.
- `credentials::tests::native_store_round_trip -- --exact --ignored`: passou no Credential Manager do Windows; usou e removeu uma entrada sintética.
- Recuperação após `process::exit(86)` comprovada em três etapas: intenção durável, estado CRDT gravado e Markdown gravado.
- A execução Linux/macOS e o CI remoto ainda precisam ser comprovados. B01, B02, B04, B05 e B06 continuam pendentes; os testes desses itens serão incorporados ao mesmo CI.
