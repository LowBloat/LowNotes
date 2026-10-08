# Backlog proposto para o LowNotes

Investigação iniciada em 07/10/2026 e revisada em 08/10/2026. Base publicada: versão v0.3.3 (`caf91b8`). A branch `codex/p1-reliability` inclui referências, criação durável, lixeira/versões/retenção (`16151d7`) e a etapa de coordenação de vínculos e texto recebido. Priorização proposta para preservar a leveza, o funcionamento offline e o controle local dos dados. Os avanços da branch e do workspace abaixo ainda não devem ser confundidos com recursos entregues na versão publicada. A execução autorizada se limita aos oito P1 originais (B01–B08).

Evidências existentes da etapa de referências: 130 testes nativos passaram no Windows, com 6 fixtures/testes condicionais ignorados na execução geral; 92 testes do frontend, verificações Svelte/TypeScript, build e 13 cenários de interface passaram. As evidências adicionais e pendências estão registradas no acompanhamento P1.

O levantamento foi feito por inspeção do README, frontend, backend, testes, empacotamento e workflows. O CI da criação durável (`37780774528`, sobre `7b7a797`) e o do histórico/lixeira (`37788217531`, sobre `16151d7`) passaram em Windows, Linux e macOS. Este último inclui 150 testes nativos, 92 testes do frontend, 19 cenários de interface, verificações Svelte/TypeScript e build de produção. A etapa posterior de coordenação de texto passou 158 testes nativos locais e aguarda sua validação remota. Não há novas medições de desempenho. Os testes nativos cobrem exclusão/edição/renomeação offline com dois e três endpoints reais, recuperação após encerramento abrupto, restauração colaborativa, retenção e preservação de cópias para revisão. O histórico e os critérios ainda pendentes estão em [P1-IMPLEMENTATION.md](F:/Desenv/2026/8-LOWCARB/3-lownotes/P1-IMPLEMENTATION.md).

Esforço relativo: **P** = mudança localizada; **M** = vários fluxos ou componentes; **G** = mudança de arquitetura, protocolo ou plataforma. As estimativas não representam prazos. **P1** = confiabilidade e sustentação; **P2** = melhorias de uso e escala; **P3** = expansão.

## Estado do trabalho local

| Item | Estado observado | O que ainda precisa ser entregue |
| --- | --- | --- |
| B01 | Concluído: catálogo durável, peers offline, confirmações, conflitos, pastas e criação recuperável; CI `37780774528` passou nos três sistemas | Manter as regressões; os fluxos de texto restantes continuam em B03 |
| B02 | Concluído: identidade, histórico, referências e mapa acompanham os movimentos; cenários offline reais e CI nos três sistemas passaram | Manter as regressões nas próximas etapas |
| B03 | Coordenação de gravações/criações/movimentos, remoção de referências e recebimento de Markdown legado; cache reimporta estados recuperados e a sincronização importa edições externas antes do manifesto; 158 testes nativos locais passaram | Validar esta etapa no CI dos três sistemas antes de marcar concluído |
| B04 | Concluído: lixeira selecionável, versões por identidade, comparação/combinação, restauração colaborativa e retenção; CI `37788217531` passou nos três sistemas | Manter as regressões nas próximas etapas |
| B05 | Concluído: união de operações, remoção durável e migração; testes locais e CI nos três sistemas passaram | Manter as regressões, incluindo a associação por identidade implementada em B02 |
| B06 | Sem benchmark reproduzível identificado | Medir RAM, CPU e latências por plataforma, incluindo WebView |
| B07 | CI de PR/main e testes antes das builds de release configurados; histórico/lixeira passou com 19 cenários de interface nos três sistemas | Validar a coordenação de texto, os benchmarks e a integração final |
| B08 | Concluído: credenciais do SO, migração, proteção de identidade e retentativa | Manter as regressões nos três sistemas |
| B27 | Ícones regenerados no workspace | Automatizar a conferência e entregar os novos ícones em uma release |

Os demais itens são propostas ou lacunas ainda não resolvidas. Uma base implementada não equivale ao atendimento de todos os critérios de aceite.

## Próximas entregas recomendadas

| Ordem | Entrega | Benefício | Esforço |
| --- | --- | --- | --- |
| 1 | Validar a etapa final de coordenação de texto (B03), com regressões B07 | Proteger Markdown e estado colaborativo na remoção de vínculos e em recebimentos legados | M |
| 2 | Benchmark nativo reproduzível (B06) | Sustentar o diferencial de leveza com medições de todo o aplicativo | M |
| 3 | Conferir a integração final dos P1 originais (B07) | Detectar regressões antes de publicar uma versão | M |
| 4 | Backup completo e exportação Markdown com imagens (B09/B19) | Recuperar o vault e usar seus arquivos em outros editores | M |
| 5 | Busca por conteúdo, índice incremental e watcher (B10/B12) | Encontrar informação e acompanhar edições externas sem reabrir notas | M/G |
| 6 | RAG mais preciso, resposta em streaming e revisão dos vínculos da IA (B11/B13/B14) | Tornar o assistente mais previsível, rápido e controlável | M |
| 7 | Mapa escalável, backlinks e tarefas consolidadas (B15/B17/B18) | Ajudar a navegar e acompanhar um vault crescente | M/G |
| 8 | Adaptadores de nuvem e canais Linux mantidos (B23/B24) | Expandir sincronização e simplificar instalação/atualização | G |

B01, B02, B04, B05 e B08 já têm implementação e evidências; entram na manutenção das regressões. B03 aguarda a validação remota da etapa de texto. B06 continua sem relatório reproduzível e B07 aguarda a integração final. As propostas de expansão devem usar a mesma semântica de identidade, exclusão e conflito já adotada no P2P.

O [CI do histórico e da lixeira](https://github.com/LowBloat/LowNotes/actions/runs/37788217531), sobre `16151d7`, concluiu com sucesso em Windows, Linux e macOS. Na etapa seguinte, passaram localmente 158 testes Rust, com 9 fixtures condicionais/workers ignorados na execução geral, e 92 testes do frontend. Os testes pais executaram 14 encerramentos reais adicionais: oito na remoção de vínculos e seis no recebimento/criação de texto legado. A execução nativa final está em `.git/p1-text-integration.txt`. Nenhum novo P1 foi criado.

## P1 — Confiabilidade e sustentação

### B01. Registrar exclusões para sincronizar com dispositivos offline — G

Na versão publicada, a exclusão era enviada aos peers conectados enquanto a reconciliação solicitava arquivos ausentes, criando risco de ressurreição de notas e perda de edições concorrentes. Na branch, o catálogo causal está integrado aos comandos e ao protocolo `/5`: é mesclado e aplicado antes do manifesto de arquivos, com identidade por nota e cópias para revisão. Os cenários offline com dois/três endpoints Iroh reais e o CI nos três sistemas passaram; B01 está concluído. Clientes antigos não trocam esse catálogo e precisam ser atualizados para convergir as operações estruturais.

Implementar registros persistentes de exclusão, confirmação por dispositivo e regras para o conflito entre excluir e editar. Fazer esse registro funcionar também para pastas e renomeações.

A lacuna identificada na revisão foi corrigida na etapa seguinte: notas, pastas e rascunhos do assistente registram intenção durável antes de publicar identidade e conteúdo. Interrupções, reutilização de nome excluído, movimentos/exclusões recebidos durante a criação e uma gravação concorrente com movimento têm regressões nativas. A etapa de criação passou 138 testes locais e seu CI nos três sistemas. A coordenação dos demais fluxos de escrita continua em B03.

**Aceite:** com dois e três dispositivos, uma nota excluída enquanto um peer está offline não reaparece silenciosamente; uma edição concorrente é preservada para revisão. Base: [comando de exclusão](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/commands.rs:282), [recebimento de exclusões](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/network.rs:495), [reconciliação](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/network.rs:802) e [catálogo local em desenvolvimento](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/catalog.rs).

### B02. Renomear e mover sem perder vínculos ou histórico — G

Na versão publicada, a renomeação movia o arquivo e removia o estado CRDT do caminho antigo. A branch passa por uma movimentação recuperável que conserva histórico e identidade; a gravação nativa de texto também preserva o CRDT. O comando sincroniza o catálogo em vez de transmitir uma exclusão do caminho antigo, e o protocolo identifica a nota independentemente do nome. Vínculos manuais e do assistente têm extremos por identidade; a projeção acompanha os dois extremos após movimentos offline. Referências Markdown, definições e wikilinks são atualizados com posições colaborativas estáveis, mantendo rótulos, títulos, âncoras e exemplos de código. Um cenário com três endpoints reais combina movimentos offline e edição de texto sem duplicar destinos ou criar um falso conflito. O CI `37722631096` sobre `5b4aec3` passou nos três sistemas; B02 está concluído e suas regressões serão mantidas nas próximas etapas.

Preservar a identidade e o histórico da nota, atualizar links relativos, wikilinks e vínculos manuais, e definir a operação correspondente no protocolo. Cobrir também a renomeação de pastas.

**Aceite:** renomear uma nota ligada a outras mantém os vínculos, a edição colaborativa e a convergência de peers offline. Base: [comando de renomeação](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/commands.rs:267) e [movimentação recuperável local](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/structural.rs:332).

### B03. Gravação atômica e recuperação de arquivos locais — M

Na versão publicada, Markdown, snapshots CRDT, configurações e vínculos usavam gravações diretas. Na branch, esses fluxos e o histórico do chat já usam substituição atômica, backup validado, preservação de arquivos corrompidos e aviso na interface. A intenção durável por nota coordena Markdown e CRDT e tem testes de recuperação após encerramento abrupto do processo. Movimentações recuperáveis foram acrescentadas no workspace e têm testes de interrupção por etapa; ainda falta completar todos os fluxos estruturais.

Concluir essa integração e coordenar a recuperação entre Markdown e CRDT. Cobrir falhas entre a gravação do estado colaborativo e do texto, além de falhas no disco e recuperação sem backup válido. Exibir a recuperação ao usuário sem descartar silenciosamente configurações e vínculos.

A criação comum e os rascunhos da IA usam uma criação durável compartilhada, validada por 21 encerramentos reais e CI nos três sistemas. A etapa seguinte coordena também a remoção de referências ao remover vínculos e o recebimento de Markdown legado, com mais 14 encerramentos reais. O cache importa estados recuperados sem duplicar texto e a sincronização importa edições externas antes dos manifestos. Esta etapa precisa passar em seu próprio CI antes de considerar B03 concluído.

**Aceite:** interrupção durante gravação, criação de nota/pasta ou salvamento de rascunho deixa uma versão válida recuperável e identidade coerente. Falta de espaço ou destino bloqueado preservam os dados anteriores e exibem erro. A recuperação não perde a identidade P2P nem substitui dados corrompidos por defaults sem aviso. Base: [gravação e recuperação](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/storage.rs), [criação de notas](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/vault.rs:174), [rascunhos do assistente](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/assistant.rs:241), [CRDT](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/crdt.rs:59), [configurações](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/config.rs:460) e [vínculos](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/links.rs:90).

### B04. Lixeira e histórico persistentes, com restauração — M/G

Na versão publicada, o desfazer usava snapshots em memória, limitados a 25 entradas e 128 MB, que desapareciam ao encerrar o aplicativo. A etapa P1 conserva excluídos e seus estados CRDT em `.lownotes/trash`, oferece seleção na interface e restaura após reinício sem sobrescrever uma nota nova. Versões em `.lownotes/versions` seguem a identidade, agrupam digitação por minuto e são restauradas como novas edições. A interface compara versões ou cópias de conflito e permite escolher/combinar/editar o resultado; alterações concorrentes desde a comparação são recusadas sem descartar o resultado digitado. Retenção é configurável, com padrão infinito, limpeza periódica/prévia/manual e proteção de peers sem confirmação, arquivos externos e junctions/symlinks. O CI desta etapa passou nos três sistemas, incluindo 150 testes Rust e 19 cenários de interface. Bases: [histórico](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/note_history.rs), [retenção](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/retention.rs) e [interface](F:/Desenv/2026/8-LOWCARB/3-lownotes/src/lib/components/HistoryModal.svelte).

Implementado e validado pelo CI `37788217531` sobre `16151d7` nos três sistemas: lixeira em disco, retenção configurável, versões de notas e comparação antes de restaurar. Restauração cria uma nova edição e mantém a sincronização coerente; B04 está concluído na branch.

**Aceite:** uma exclusão pode ser revertida após reiniciar; o usuário pode comparar uma nota com sua cópia de conflito e escolher ou combinar os textos. Base: [restauração persistente](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/commands.rs:295).

### B05. Mesclar vínculos do mapa criados em dispositivos diferentes — G

Na base v0.3.3, os vínculos manuais e do assistente ficam em `.lownotes/links.json` e usam hash/data de modificação. O trabalho P1 adicionou o histórico imutável `.lownotes/link-operations.json`, com união de adições/remoções, proteção contra replay de listas antigas e preservação de origens. Os testes locais com três dispositivos e o CI `37686376522` nos três sistemas passaram. Este item está concluído; a integração com os caminhos atualizados em renomeações continua em B02.

Adotar operações de adicionar/remover vínculos com identidade e resolução de concorrência, preservando a origem de cada relação.

**Aceite:** dois dispositivos criam vínculos distintos offline e ambos continuam presentes após sincronizar; remover um vínculo não é revertido por uma cópia antiga do arquivo. Base: [metadados de vínculos](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/vault.rs:243) e [recebimento da sincronização](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/network.rs:888).

### B06. Medir e proteger o consumo de RAM, CPU e a latência — M

A leveza é um diferencial do produto, mas não encontrei um benchmark reproduzível no projeto. O número de RAM informado pelo autor precisa ter um cenário de referência documentado.

Medir o processo principal e o conjunto de processos do aplicativo, incluindo WebView, em Windows, Linux e macOS. Separar modelos locais externos da medição do aplicativo. Registrar abertura, digitação, busca, mapa, imagens e sincronização em vaults de 100, 1.000 e 10.000 notas.

**Aceite:** existe um relatório reproduzível com cenário, plataforma, RAM, CPU e tempos; mudanças podem ser comparadas com a mesma base. Não impor uma meta numérica antes de medir.

### B07. Verificações em cada PR e regressões de integração — M

Na branch, o workflow de PR/push em `main` executa frontend, Rust, credenciais nativas e Playwright em Windows, Linux e macOS. A release também exige testes Rust antes de cada build nativa e regressões de interface na verificação inicial. Referências, criação durável e histórico/lixeira (`37788217531`, sobre `16151d7`) passaram na matriz dos três sistemas. Falta validar remotamente a etapa de coordenação de texto, acrescentar os relatórios nativos de desempenho e conferir a integração final dos P1.

Automatizar os cenários completos de exclusão offline, renomeação, interrupção de gravação e restauração. Manter as regressões existentes de imagens, edição concorrente, tarefas, busca, temas, atalhos e exportação. Distinguir testes de interface com IPC simulado dos testes de persistência, credenciais e rede nativas. Se necessário, ampliar o gatilho de push para branches de desenvolvimento sem PR.

**Aceite:** uma regressão do backend ou desses fluxos é detectada antes da criação de uma release. Base: [release](F:/Desenv/2026/8-LOWCARB/3-lownotes/.github/workflows/release.yml:30) e [CI em andamento](F:/Desenv/2026/8-LOWCARB/3-lownotes/.github/workflows/ci.yml:1).

### B08. Armazenar credenciais fora do JSON de preferências — M

Na base v0.3.3, chaves de provedores e identidade P2P eram serializadas no JSON. O trabalho P1 já migrou esses dados para o armazenamento de credenciais do SO, com referências imutáveis no JSON, verificação antes de limpar dados antigos e retentativa quando o armazenamento está bloqueado. Migração, exportação e preservação de identidade foram testadas; os backends nativos Windows, Linux e macOS passaram.

Usar o armazenamento de credenciais do sistema, migrar instalações existentes e definir o comportamento quando esse armazenamento estiver indisponível. Exportações de configurações devem omitir segredos.

**Aceite:** `settings.json` não contém as credenciais em texto aberto e a migração preserva os provedores e a identidade dos dispositivos. Base: [configuração local](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/config.rs:469).

## P2 — Uso diário e escala

### B09. Backup e restauração do vault completo — M

Oferecer backup manual e agendado de Markdown, `.lownotes`, imagens e histórico colaborativo. Usar uma cópia consistente do SQLite e validar o pacote antes de restaurar. Explicar que conversas do assistente estão em outro diretório e oferecer sua inclusão sem credenciais.

**Aceite:** restaurar em uma instalação limpa reproduz notas, imagens e vínculos; dados existentes não são sobrescritos sem revisão. Depende de B03 e deve considerar B04.

### B10. Índice local incremental para busca e RAG — M/G

A listagem lê notas para extrair títulos, o RAG volta a ler os arquivos a cada consulta e o manifesto relê conteúdos para calcular hashes. A busca da sidebar filtra somente títulos e caminhos.

Criar um índice local compartilhado com atualização por arquivo alterado, limites de cache e reconstrução segura. Disponibilizar busca global pelo conteúdo, com trechos destacados e navegação até a ocorrência. Reutilizar o índice na recuperação de contexto da IA.

**Aceite:** buscar uma expressão contida apenas no corpo encontra a nota; a alteração de uma nota não exige reindexar o vault inteiro. Base: [listagem](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/vault.rs:63), [RAG](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/rag.rs:76) e [filtro da sidebar](F:/Desenv/2026/8-LOWCARB/3-lownotes/src/lib/note-tree.ts:23).

### B11. Melhorar a qualidade e o controle do RAG — M

O RAG atual é lexical, pontuando palavras, frases e títulos. A seleção usa cinco trechos e limita o contexto por quantidade de caracteres; a segmentação não acompanha integralmente a sintaxe Markdown.

Criar um conjunto de perguntas e resultados esperados; tratar acentos e variações de termos; segmentar respeitando seções, listas e blocos de código; limitar contexto conforme o modelo. Permitir escolher pastas e notas, excluir conteúdo e revisar os trechos enviados. Avaliar recuperação semântica apenas como opção posterior, com medição de custo e qualidade.

**Aceite:** uma bateria de consultas mostra melhora mensurável e o contexto não corta blocos relevantes arbitrariamente nem excede o limite configurado. Base: [segmentação e ranking](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/rag.rs:123) e [montagem de contexto](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/commands.rs:642).

### B12. Detectar alterações feitas por outros programas — M

O backend reconhece divergências entre Markdown e CRDT quando o documento é carregado ou sincronizado. Não encontrei um watcher dedicado do filesystem que mantenha a interface atualizada imediatamente.

Observar alterações externas e atualizar sidebar, nota aberta, índice e mapa. Diferenciar alterações do próprio LowNotes e preservar edições locais em caso de concorrência.

**Aceite:** salvar pelo VS Code ou outro editor atualiza o LowNotes sem reabrir a nota e sem substituir silenciosamente mudanças locais. Base: [reconciliação com o arquivo](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/crdt.rs:108).

### B13. Respostas da IA em streaming e botão de cancelar — M

A chamada atual espera uma resposta JSON completa, com timeout de 180 segundos, e a interface mantém a conversa em estado de carregamento.

Mostrar a resposta conforme chega e cancelar a operação no backend. Preservar respostas parciais como incompletas, sem aplicar propostas incompletas de arquivos ou vínculos.

**Aceite:** o usuário vê progresso textual e consegue interromper a chamada; cancelar não gera uma alteração acidental. Base: [chamada ao modelo](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/rag.rs:264).

### B14. Revisar vínculos sugeridos pela IA antes de aplicar — P/M

As edições de texto já são revisáveis. Entretanto, os blocos `lownotes-links` da resposta podem ser aplicados diretamente; remover um vínculo também pode alterar a referência escrita no Markdown.

Mostrar as relações propostas e as mudanças de texto decorrentes, permitindo aceitar ou recusar cada uma. Registrar e permitir desfazer o lote aplicado.

**Aceite:** nenhum lote de vínculos da IA muda notas ou o mapa antes da revisão, exceto quando o usuário habilitar explicitamente aplicação automática. Base: [aplicação na conversa](F:/Desenv/2026/8-LOWCARB/3-lownotes/src/lib/components/AiChatSidebar.svelte:209).

### B15. Mapa fluido para vaults grandes e navegação por contexto — M/G

O layout usa 180 iterações com comparação entre pares de nós, executadas no frontend. Isso indica custo quadrático a avaliar em vaults maiores.

Mover o cálculo para fora da thread da interface, evitar recalcular posições desnecessariamente e limitar rótulos conforme o zoom. Adicionar filtros por pasta, origem do vínculo e distância da nota selecionada. As posições manuais já são persistidas localmente; preservar esse comportamento.

**Aceite:** o mapa responde a arrastar e ampliar durante o cálculo e permite explorar só a vizinhança de uma nota. Avaliar em B06. Base: [layout](F:/Desenv/2026/8-LOWCARB/3-lownotes/src/lib/components/GraphView.svelte:100).

### B16. Realce das linguagens dentro dos blocos de código — M

O editor configura Markdown sem uma coleção de parsers de linguagens para os blocos cercados. Corrigir a cor do nome da linguagem não equivale a realçar sua sintaxe.

Carregar parsers sob demanda para linguagens comuns e usar cores legíveis em todas as paletas. Manter blocos desconhecidos como texto e avaliar realce na visualização/exportação.

**Aceite:** Rust, Go, Python, JavaScript e SQL têm realce adequado sem carregar todos os parsers na abertura. Base: [configuração do editor](F:/Desenv/2026/8-LOWCARB/3-lownotes/src/lib/components/Editor.svelte:403).

### B17. Backlinks e autocompletar links entre notas — M

O mapa e a resolução de wikilinks já existem. Adicionar uma lista das notas que apontam para a nota aberta, sugestão de destinos ao escrever `[[` e identificação de referências quebradas ou ambíguas.

**Aceite:** nomes duplicados exibem seus caminhos; navegar entre referências dispensa abrir o mapa; referências inválidas são visíveis. Depende de B02 e se beneficia de B10.

### B18. Visão consolidada de tarefas — M

Os checkboxes já são interativos. Reutilizar seu mapeamento para listar tarefas pendentes e concluídas do vault, com filtros por nota e pasta e navegação para a linha original.

**Aceite:** marcar uma tarefa nessa visão altera o Markdown original, participa do desfazer e sincroniza com os peers. Não exigir um banco separado de tarefas.

### B19. Exportar Markdown com imagens portáveis — M

As referências `lownotes-image:` dependem do banco de imagens do LowNotes. Word e PDF já resolvem essas imagens; falta uma exportação de Markdown que abra com imagens em outros editores.

Exportar uma cópia das notas com os arquivos de imagem em uma pasta de anexos e reescrever os caminhos relativos nessa cópia. Incluir exportação de uma nota e de um vault.

**Aceite:** o pacote exportado abre com imagens em um editor Markdown externo, sem modificar o vault original. Base: [comportamento documentado](F:/Desenv/2026/8-LOWCARB/3-lownotes/README.md:51).

### B20. Gestão do espaço ocupado por imagens — M

O armazenamento deduplica imagens e preserva blobs após apagar notas ou desfazer colagens. Isso protege histórico e peers offline, mas pode aumentar o tamanho do vault ao longo do tempo.

Exibir uso de espaço e referências de cada imagem. Oferecer limpeza com prévia, retenção e recuperação; considerar lixeira, versões e peers offline antes de remover blobs. Evitar limpeza automática baseada apenas nas notas atualmente visíveis.

**Aceite:** uma limpeza não quebra versões restauráveis nem imagens necessárias após reconectar outro dispositivo. Depende de B01 e B04.

### B21. Diálogos consistentes e navegação rápida — P/M

A sidebar ainda usa `prompt`, `confirm` e `alert` para operações de arquivos. Unificar esses fluxos com validação, indicação de destino e mensagens no contexto. Acrescentar uma paleta de comandos, favoritos, notas recentes e atalhos descobríveis.

**Aceite:** criar, mover e renomear apresentam erros sem perder o texto digitado e funcionam integralmente pelo teclado. Base: [operações da sidebar](F:/Desenv/2026/8-LOWCARB/3-lownotes/src/lib/components/Sidebar.svelte:82).

### B22. Limites de acesso ao vault e política de conteúdo — M

`safe_join` rejeita caminhos absolutos e `..`, mas não resolve links simbólicos ou junctions nos componentes do caminho. A CSP está desativada. Por outro lado, o renderizador já desativa HTML bruto e o Mermaid usa modo estrito; preservar essas proteções.

Validar o destino efetivo das operações contra a raiz do vault, definir comportamento para links simbólicos e ativar uma CSP compatível com diagramas, fontes e imagens. Revisar limites e permissões dos comandos nativos.

**Aceite:** caminhos que resolvem para fora do vault são recusados, enquanto recursos legítimos de preview e exportação continuam funcionando. Base: [validação de caminhos](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/vault.rs:41) e [CSP](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/tauri.conf.json:24).

### B28. Documentação consistente e guia de diagnóstico — P/M

O README da base v0.3.3 declarava `/2` enquanto o backend usava `/3`. Essa indicação foi corrigida no trabalho P1: agora documenta `/5`, os fallbacks, as limitações estruturais de clientes antigos e o prefixo dos códigos de pareamento. A tabela inicial ainda apresenta Megumin e Rimuru sem destacar a paleta LowBloat padrão; falta também o guia de diagnóstico abaixo.

Revisar documentação junto às releases; explicar compatibilidade, backup completo, localização dos dados e diferenças entre RAG lexical local e modelos locais. Adicionar um guia de diagnóstico de pareamento, sincronização e atualizações com informações que possam ser compartilhadas sem chaves, códigos de pareamento ou conteúdo das notas.

**Aceite:** informações de versão e compatibilidade correspondem ao código; o usuário consegue diagnosticar uma falha sem expor suas notas ou credenciais. Base: [README](F:/Desenv/2026/8-LOWCARB/3-lownotes/README.md:113) e [protocolo](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/network.rs:23).

## P3 — Expansão

### B23. Adaptadores de sincronização com armazenamento externo — G

Adicionar uma interface comum e implementar provedores em etapas: S3/WebDAV como opções e Google Drive/OneDrive conforme prioridade do produto. Tratar autenticação, retentativas, estado offline, exclusões e conflitos.

Sincronizar operações/metadados e blobs de imagem, preservando o armazenamento local. Não copiar um SQLite aberto entre dispositivos como estratégia de sincronização.

**Aceite:** alterações concorrentes e exclusões convergem sem perda de dados, com estados de progresso e erro por provedor. Depende de B01, B02, B03, B05 e B09.

### B24. Distribuição e atualização por canais Linux — M/G

Já existem pacotes e detecção do formato instalado. Avaliar publicar e manter canais AUR e Flathub, além de repositórios para DEB/RPM conforme demanda. Verificar a existência de canais externos antes de criar duplicatas, porque esta inspeção não consultou catálogos públicos.

Automatizar metadados, checksums e publicação; validar permissões de filesystem, tray e P2P no ambiente empacotado.

**Aceite:** a instalação e a atualização usam o canal escolhido pelo usuário sem substituir um pacote nativo por AppImage. Base: [política de atualização](F:/Desenv/2026/8-LOWCARB/3-lownotes/src-tauri/src/updates.rs:10) e [pacote Arch](F:/Desenv/2026/8-LOWCARB/3-lownotes/packaging/arch/PKGBUILD:1).

### B25. Cliente para celular — G

Começar por leitura, edição, imagens e tarefas com sincronização, priorizando a adaptação da interface, pareamento e comportamento de conexão em segundo plano. Avaliar as limitações de cada plataforma antes de prometer equivalência com o desktop.

**Aceite:** o mesmo conjunto de notas pode ser usado e editado entre desktop e celular, sem exigir que a IA esteja habilitada. Depende da estabilização de B01–B05.

### B26. Modelos de notas — P/M

Adicionar modelos opcionais para notas diárias, reuniões, estudos e acompanhamento de projetos, com campos simples para título e data e seleção da pasta de destino.

**Aceite:** criar uma nota por modelo não sobrescreve notas existentes, e os modelos podem ser personalizados sem dependência de IA.

### B27. Geração consistente dos assets da marca — P

Automatizar a geração e a conferência dos ícones a partir da logo canônica, evitando que a tela de boas-vindas e os pacotes voltem a usar marcas diferentes.

**Aceite:** atualizar a logo canônica gera os assets corretos de todas as plataformas e uma inconsistência é detectada antes da release. A substituição dos ícones antigos já foi feita no workspace; falta entregar essa alteração em uma versão.

## Ordem sugerida

1. Validar o CI da coordenação dos fluxos de texto (B03), mantendo as regressões já concluídas de B01/B02/B04/B05/B08.
2. Criar os relatórios nativos de medição (B06) e conferir a integração final (B07), dentro dos P1 originais.
3. Após concluir os P1, oferecer backup consistente (B09) e exportação Markdown portável (B19).
4. Melhorar busca, RAG, observação de arquivos e resposta da IA (B10–B14).
5. Evoluir mapa, código, organização, tarefas e portabilidade (B15–B22).
6. Implementar nuvem e novas plataformas sobre essa base (B23–B25). Modelos, automação da marca e documentação podem entrar antes como entregas menores de B26, B27 e B28.
