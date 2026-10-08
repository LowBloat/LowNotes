# Implementação dos itens P1

Escopo: B01–B08 de BACKLOG.md, solicitado em 07/10/2026. Este arquivo acompanha trabalho e evidências; um item só deve ser marcado concluído depois de validar seus critérios de aceite.

- [ ] B01 — exclusões duráveis, peers offline, confirmação, conflito excluir/editar, pastas.
- [x] B02 — identidade e histórico em renomeações, referências e mapa, convergência offline; CI `37722631096` nos três sistemas.
- [ ] B03 — gravação atômica, backups válidos, recuperação coordenada e aviso ao usuário.
- [ ] B04 — lixeira e versões persistentes, retenção, comparação e restauração colaborativa.
- [x] B05 — operações de vínculos com mesclagem e remoção durável.
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
- B01/B02: catálogo causal com IDs estáveis, exclusões, restaurações e confirmações por dispositivo. O protocolo `/5` troca o catálogo e aplica operações antes dos manifestos; pacotes de conteúdo e edição carregam a identidade da nota. Há fallback `/4`, `/3` e `/2`, com proteção de alterações legadas ambíguas. A renomeação nativa conserva a história CRDT em vez de removê-la; os vínculos manuais e do assistente agora têm extremos por identidade, sem modificar as adições originais. A etapa posterior acrescentou referências escritas; ver evidências abaixo.
- B03/B04: projeção estrutural com intenção durável, retirada de todos os arquivos afetados antes de colocar destinos, conservação de arquivos da pasta e arquivos removidos em `.lownotes/trash`. O desfazer de exclusão usa essa base persistente, inclusive depois de reiniciar. Faltam a tela de lixeira, retenção configurável, versões de notas e comparação/restauração de versões como nova edição colaborativa.
- B01/B07: o editor envia o ID da nota e do vault; uma edição atrasada após exclusão solicita um snapshot completo para revisão, sem editar uma nota nova criada no mesmo caminho. O cenário de interface usa IPC isolado; a conservação e o roteamento nativos são testados em Rust.
- Ícones novos e o backlog previamente criados permanecem no workspace; não são usados como prova de conclusão dos P1.

## Evidências locais (Windows)

- `bun test`: 92 testes passaram.
- `bun run check` e `bun run check:e2e`: sem erros.
- `bun run build`: compilação concluída.
- `bun run test:e2e -- --workers 2`: 13 cenários passaram, incluindo recuperação do editor atrasado com os IDs originais, tarefas/desfazer/refazer com texto remoto, busca nos três modos, links, todas as paletas/modos, recuperação e credenciais indisponíveis.
- `cargo test --locked --lib --manifest-path src-tauri/Cargo.toml`: 130 testes passaram no Windows; 6 fixtures/testes condicionais ficaram ignorados na execução geral. Os workers de crash são executados pelos testes pais.
- `credentials::tests::native_store_round_trip -- --exact --ignored`: passou no Credential Manager do Windows; usou e removeu uma entrada sintética.
- Recuperação após `process::exit(86)` comprovada em três etapas: intenção durável, estado CRDT gravado e Markdown gravado.
- B05: teste com três endpoints Iroh reais, adições e remoção offline, timestamps invertidos e replay de snapshots/listas antigos. Inclui concorrência local de 20 ações, reconstrução de uma projeção antiga, validação de mensagens, conservação de origens, um histórico acima de 10 MB e fallback de imagens para `/3`.
- B01: três endpoints Iroh reais com exclusão de pasta, edição e renomeação offline; a exclusão converge, a edição fica em uma única cópia de revisão e uma nova nota no caminho antigo tem identidade independente. Confirmações dos endpoints são persistidas. Pacotes atrasados, integridade e associação entre caminho/identidade têm regressões nativas.
- B01: dois endpoints reais com um filho criado offline e desconhecido pelo dispositivo que excluiu a pasta; o conteúdo é preservado para revisão, sem reaparecer dentro da pasta excluída. Fallback `/4` mantém operações de vínculos sem receber pacotes estruturais.
- B03: `process::exit(86)` em sete etapas da projeção estrutural, incluindo a fila durável antes da publicação do catálogo e a interrupção entre catálogo e intenção de filesystem comprova recuperação de trocas simultâneas de nomes e estados CRDT. Arquivo recriado externamente após staging é preservado sem desfazer a exclusão. Movimentações locais têm regressões adicionais para pastas, assets e edições CRDT criadas antes da mudança.

## Evidências remotas e pendências

- CI `37671886130` sobre `d2e3177`: Linux e macOS concluíram Rust, credenciais nativas e os 11 cenários de interface. Windows teve uma falha de timestamp no cenário de atualização incremental, corrigida e validada localmente; os testes nativos do Windows já haviam passado em execução local com uma entrada sintética.
- B08 está comprovado por testes de migração/indisponibilidade/exportação e pelos backends nativos dos três sistemas. Nenhuma configuração real do usuário foi usada pelos testes.
- CI `37686376522` sobre `d74969b` concluiu com sucesso nos três sistemas, incluindo Rust, credenciais nativas e interface. B05 está concluído sobre essa evidência e seus critérios de mesclagem e remoção durável; a nova integração estrutural `/5` ainda precisa passar em seu próprio CI remoto.
- B01 e B03 avançaram localmente e aguardam auditoria completa e a nova verificação multiplataforma. Naquela etapa, B02 ainda precisava atualizar referências escritas; os vínculos do mapa já acompanham as identidades e têm regressões de remoção, restauração e recriação de nomes. B04 ainda precisa da interface, retenção e histórico de versões. B06 permanece pendente. B07 aguarda todas essas regressões e a execução remota da nova alteração.

## Etapa de identidades do mapa e auditoria estrutural

- CI `37717262436` sobre `e43e5e6` concluiu com sucesso em Windows, Linux e macOS: integração estrutural, credenciais nativas e interface.
- B02: vínculos têm associação imutável entre cada adição e os IDs das duas notas. A projeção segue renomeações/movimentações, inclusive de pastas; remoção conserva a proteção contra listas antigas. Recriar um nome não transfere o vínculo antigo, mas permite criar uma relação explícita com a nova identidade. Restauração revela o vínculo original.
- Teste com três endpoints Iroh reais combina movimentos offline de pasta, nota de origem e nota de destino, adição de vínculo e remoção posterior com replay de metadados antigos. Outra regressão recusa reassociar um vínculo sem alterar o arquivo local.
- B03: `.lownotes/pending-catalog.json` precede a publicação do catálogo e permite retomar a projeção mesmo quando nenhum journal de filesystem chegou a ser criado. Encerramento abrupto em sete fases passou localmente. Dados staged sem uma intenção válida ficam preservados e produzem erro/aviso.
- B01/B03/B04: pastas vazias recebidas são materializadas; restauração escolhe outro nome quando o original já pertence a uma nova identidade. Um arquivo/pasta recriado após staging ou placement é preservado: Markdown vai para revisão e os arquivos completos ficam em `.lownotes/recovered-files/`.
- Pendências continuam: referências escritas em renomeações; interface de lixeira/versões, retenção e comparação/restauração; benchmarks nativos dos três sistemas; auditoria das criações interrompidas e da coordenação de todas as mutações locais; validação remota desta nova etapa. Os P1 permanecem abertos onde esses critérios faltam.
- B03: substituição atômica faz retentativas limitadas quando o Windows mantém um handle sem permissão de exclusão; reutiliza o mesmo tempfile sincronizado e não remove o destino. Testes com handles reais cobrem liberação breve e bloqueio persistente, conservando a versão original e o backup. A execução final desta etapa passou 117 testes no Windows; os dois testes de handles são específicos dessa plataforma.

## Etapa de referências escritas (08/10/2026)

- CI `37719161090` sobre `efe5fa7` concluiu com sucesso em Windows, Linux e macOS; comprova a etapa anterior de mapa e fila durável.
- B02: parser Markdown com intervalos exatos para destinos, incluindo definições compartilhadas e wikilinks. Reescritas preservam rótulos, aliases, títulos, âncoras, CRLF e exemplos de código; links relativos são recalculados para notas e assets.
- Associações entre referências, notas e posições do texto ficam no estado Yrs, com coordenadas UTF-16 compatíveis com Yjs e tradução para os offsets UTF-8 do parser. Operações determinísticas eliminam URLs duplicadas de reescritas concorrentes. Destinos deliberadamente alterados não são revertidos e podem ser associados à nova identidade.
- A manutenção estrutural das referências não vira um conflito de edição artificial. Um cenário com três endpoints Iroh reais combina movimentos offline de pasta/origem/destino e uma edição de texto: o texto e todos os destinos convergem, sem cópias de conflito espúrias. Os fixtures agora usam endereços locais explícitos, mantendo o protocolo e a criptografia reais sem depender de relays/descoberta públicos.
- B03: encerramento real do processo em cinco etapas da movimentação conserva referências de entrada/saída e Markdown/CRDT após reiniciar. Há uma regressão adicional para definições no fim do arquivo seguidas de uma edição posterior.
- Referências atualizadas são notificadas ao editor aberto pelo mecanismo de eventos CRDT. Grafo e visualização compartilham a distinção entre Markdown relativo e wikilink a partir do vault, com decoding dos caminhos. URLs web na visualização usam o navegador do usuário.
- Execução local final: `cargo test --locked --lib --manifest-path src-tauri/Cargo.toml` passou 130 testes, com 6 fixtures/testes condicionais ignorados; workers de crash são executados pelos pais. `bun test` passou 92 testes; Svelte/TypeScript e build passaram. Os 13 cenários de interface passaram, incluindo caminhos Markdown relativos codificados, wikilinks e abertura de URLs web no navegador.
- Um cenário de rede ultrapassou 30 segundos em uma execução conjunta anterior. Passou isoladamente e nas duas execuções completas subsequentes; a última matriz local com endereços explícitos passou em 9,10 segundos. Não foi necessário aumentar o timeout ou retirar o cenário.
- Escopo P1 permanece aberto: auditoria de criação interrompida e coordenação de mutações; lixeira/versões com interface, retenção, comparação e restauração colaborativa; benchmarks nativos nas três plataformas; conferência do CI desta etapa e de todas as etapas posteriores.

## Etapa de criação durável (08/10/2026)

- Escopo preservado: somente B01–B08 do backlog original. B05/B08 continuam concluídos; nenhum novo P1 foi criado.
- CI `37722631096`, sobre `5b4aec3`, concluiu com sucesso em Windows, Linux e macOS. Confirma a etapa de referências escritas, incluindo persistência/rede nativas, credenciais e os 13 cenários de interface.
- B01/B03: notas, pastas e rascunhos da IA compartilham uma intenção durável anterior à publicação de identidade e conteúdo. Notas novas têm um seed CRDT independente, inclusive quando vazias. A criação evita reaproveitar a identidade de um nome excluído.
- A recuperação segue movimentos recebidos durante uma criação e não deixa o arquivo antigo reaparecer. Uma exclusão observada continua válida; os dados e o estado colaborativo são preservados no formato normal da lixeira e a restauração funciona após reiniciar, mesmo se a nota ainda não tivesse sido materializada.
- Um arquivo externo que ocupa o nome após a intenção é mantido no lugar, com identidade independente; a criação é colocada em outro nome. Uma edição externa depois da projeção conserva também o texto original para revisão. Uma edição CRDT posterior é mantida sem criar um falso conflito.
- Gravações nativas de documentos e recebimento de conteúdo identificado/legado passam pelo coordenador estrutural. Um teste concorrente confirma que salvar o caminho antigo durante um movimento não recria esse arquivo. Criar pastas também solicita reconciliação dos peers.
- B03/B07: 21 execuções de processo filho encerradas com `process::exit(86)`: oito etapas para notas, oito para rascunhos da IA e cinco para pastas. Incluem intenção de criação, catálogo, intenção da edição, estado CRDT, Markdown, bindings e limpeza. O teste pai reabre pela listagem normal, confere conteúdo/estado/identidade e repetição idempotente.
- Falha de filesystem com um arquivo bloqueando a pasta preserva o bloqueio e a intenção; após remover o bloqueio, a recuperação conclui a criação. Os limites de conteúdo também são validados na transação por nota.
- Validação local final: `cargo test --locked --lib --manifest-path src-tauri/Cargo.toml` passou **138 testes**, sem falhas, com **7 fixtures/testes condicionais ignorados** na execução geral; os workers de crash são chamados pelos testes pais. Resultado preservado em `.git/p1-native-latest.txt` nesta checkout. Não houve mudança no frontend nesta etapa.
- Próximos P1: concluir a coordenação das mutações restantes (incluindo remoção de referências escritas), entregar a interface de lixeira/versões, retenção e comparação/restauração de B04; medir B06 nos três sistemas; validar todas essas etapas no CI de B07. Não marcar os P1 completos apenas por esta etapa passar.
