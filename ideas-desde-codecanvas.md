# Ideas para Code Canvas tomadas de otras dos apps

Listado de funciones que tienen dos apps ajenas y que Code Canvas (`C:\rysted\code-canvas`) todavía no tiene, con una nota sobre cómo encajaría cada una. Es una lista de ideas para quien las implemente: no hay nada hecho ni decidido.

- **Parte 1 (ideas 1 a 18):** [CodeCanvas](https://github.com/Lightningbulb2/CodeCanvas), de Lightningbulb2. Es la más parecida a Code Canvas.
- **Parte 2 (ideas 19 a 24):** [codex-canvas](https://github.com/DragosCP/codex-canvas), de DragosCP. Es otro tipo de app; ver la introducción de esa parte.

Fuentes:

- **Fecha de la comparación:** 17/11/2026.
- **CodeCanvas:** commit `00daa32` de `main`. Se leyeron `README.md`, `ARCHITECTURE.md`, `rust/README.md`, `vscode/README.md` y `zed/README.md`. No se leyó su código ni se ejecutó.
- **codex-canvas:** commit `e776229` de `main`. Se leyó `README.md`, la lista de archivos de `apps/desktop` y resúmenes automáticos de tres archivos (`electron/draft-workspaces.cjs`, `src/canvasProposalStaleness.ts`, `src/canvasTargeting.ts`). No se ejecutó.
- **Code Canvas:** commit `bf9e0c5`. Se leyeron `README.md`, `README.es.md`, la primera mitad de `docs/NOTES.md` y las secciones de `commit_ai.rs`, `editor.rs`, `agents.rs` y `session.rs`, más búsquedas puntuales en el código. No se ejecutó.

## Code Canvas y CodeCanvas en una frase

- **Code Canvas** muestra el repositorio entero: carpetas como cajas anidadas, archivos como tarjetas, layout automático, y curvas de símbolos y dependencias. Ya tiene editor, git, extensiones de VS Code, sesión compartida y asistentes visibles en el lienzo.
- **CodeCanvas** es una mesa de trabajo: ponés a mano los archivos que te interesan, los cortás en fragmentos que se mueven y se vuelven a unir, y cada archivo o fragmento tiene su propio chat con un modelo.

Casi toda la parte 1 sale de esa diferencia.

## Reglas de Code Canvas que condicionan las ideas

Quien implemente tiene que respetarlas o plantear el cambio al dueño del proyecto.

- **El layout es automático.** Las tarjetas las ubica `canvas-core/src/layout.rs`. No existe hoy una tarjeta en una posición elegida por el usuario.
- **La app no tiene cuentas ni claves.** `docs/NOTES.md` lo dice explícito ("There is no account in the app"). Los mensajes de commit los escribe el `claude` o el `codex` que la persona ya tiene instalado, o un comando propio (`commit_ai.rs`).
- **El código no lleva comentarios.** Lo que no se entiende leyendo va a `docs/NOTES.md`, en el mismo commit que el cambio.
- **Los escenarios de `qa/` manejan la app real.** Una función nueva debería traer los suyos.
- **El rendimiento es una función.** Nada de esto puede costar por fotograma cuando no se usa.
- Antes de empezar, leer `CONTRIBUTING.md` (no se leyó para este listado).

## Orden sugerido

1. Regiones dentro de un archivo (idea 8). Es la base del chat por fragmento.
2. Chat por archivo y por región (idea 1).
3. Borradores (idea 3).
4. Explicar archivo (idea 5).
5. Lista de contexto (idea 2).
6. Aparte, chicas e independientes: buscar con más opciones (idea 14) e instancia única (idea 17).
7. De la parte 2, lo más barato es ver qué cambió cada asistente (idea 23). El aviso de borrador desactualizado (idea 22) va junto con los borradores (idea 3).

Los tamaños que figuran en cada idea son una estimación a ojo, sin haber diseñado nada.

---

# Parte 1: CodeCanvas (Lightningbulb2)

## Modelo de lenguaje

### 1. Chat por archivo y por fragmento

- **En CodeCanvas:** cada archivo, cada tarjeta suelta y cada región tiene su propio hilo de chat, que se abre desde una pestaña al costado y se guarda con el lienzo. Tiene dos modos: mandar solo ese código, o mandar además la lista de contexto (idea 2). Un botón reinicia el hilo. Hay también un chat general del proyecto en el panel derecho.
- **En Code Canvas hoy:** no hay chat propio. Lo más parecido es el panel de la extensión oficial de Claude Code, que es una webview y funciona solo en Windows.
- **Cómo encaja:** seguir el camino de `commit_ai.rs`, que ya encuentra y lanza `claude -p`, `codex exec` o un comando propio, les pasa el pedido por la entrada estándar y lee lo que imprimen en hilos aparte. Así no hace falta ninguna clave. Hay tres cosas que `commit_ai.rs` no resuelve y el chat necesita:
  - **Streaming.** `commit_ai::write` bloquea hasta la respuesta entera. Claude Code tiene salida en streaming para el modo `-p`; verificar las opciones exactas en su documentación.
  - **Continuar un hilo.** Un hilo por tarjeta se corresponde con una sesión del asistente que se retoma en cada mensaje. Verificar cómo retoma sesiones cada programa.
  - **Dónde se guarda.** Ver "Decisiones abiertas".
- **Extra que sale gratis:** si Claude Code está conectado por hooks (`agents.rs`), el asistente que responde en un chat aparece en el lienzo con su cara, como cualquier otro.
- **Tamaño:** medio para el chat de archivo; depende de la idea 8 para el de fragmento.

### 2. Lista de contexto

- **En CodeCanvas:** una lista global de archivos y rangos de líneas que se manda junto con el pedido en modo "Global". Un panel muestra el historial de lo que entró y salió, con la cantidad de líneas de cada cosa.
- **En Code Canvas hoy:** no existe.
- **Cómo encaja:** una entrada en el menú de clic derecho de una tarjeta o de una selección ("agregar al contexto") y un panel más en la barra izquierda. La selección de archivos del explorador (las casillas) podría servir de punto de partida.
- **Tamaño:** chico, una vez que existe el chat.

### 3. Borradores

- **En CodeCanvas:** lo que sugiere el modelo no se escribe en el archivo. Llega como un borrador: una copia editable de las líneas, mostrada como diff contra el original, con Aplicar y Descartar. Los tramos largos sin cambios se pliegan.
- **En Code Canvas hoy:** no existe para sugerencias, pero el diff dentro de la tarjeta ya está hecho para git: líneas quitadas arriba de las nuevas o lado a lado, con lo que no cambió plegado (`Page::sync_diff`, `Repo::replace_source`).
- **Cómo encaja:** un borrador es el mismo diff con otro origen: en vez de comparar contra git, compara contra el texto propuesto. Aplicar es una sola edición del editor, o sea un solo paso de deshacer.
- **Tamaño:** medio.

### 4. Sugerencias como tarjetas flotantes

- **En CodeCanvas:** el código que el modelo propone para otro lugar aparece como una tarjeta suelta cerca del chat, unida con una línea punteada al lugar del archivo donde iría, con un botón que lo aplica ahí.
- **En Code Canvas hoy:** existen las curvas (`canvas-render/src/curve.rs`) pero no las tarjetas sueltas.
- **Cómo encaja:** depende de la capa de tarjetas libres (idea 9). Sin ella, la versión barata es que toda sugerencia vaya al borrador del archivo destino (idea 3) y el lienzo viaje hasta ahí, como ya hace con los cambios de un asistente.
- **Tamaño:** grande con tarjetas; chico con la versión barata.

### 5. Explicar archivo / Explicar todo

- **En CodeCanvas:** le pide al modelo una lista de segmentos (líneas, título, explicación) y convierte cada uno en una región con la explicación como nota. "Explicar todo" recorre el proyecto archivo por archivo.
- **En Code Canvas hoy:** no existe.
- **Cómo encaja:** acá Code Canvas parte con ventaja. El índice de símbolos ya sabe dónde empieza y termina cada función (`Symbol::body_end`, `symbol_at_line`), así que no hace falta pedirle al modelo que adivine los cortes: se le manda cada función y devuelve solo el texto. Las explicaciones se pueden mostrar al alejarse, sobre el cuerpo de la función, que ya se resalta.
- **Tamaño:** medio. Depende de la idea 8.

### 6. Proveedores locales y propios

- **En CodeCanvas:** llama.cpp (por defecto), Ollama, cualquier servidor compatible con OpenAI y Anthropic, por HTTP, con streaming, selector de modelo y la clave guardada en un archivo.
- **En Code Canvas hoy:** `commit_ai.rs` cubre Claude Code, Codex y `custom` (cualquier comando, por ejemplo `ollama run llama3`).
- **Cómo encaja:** `custom` ya da modelos locales sin clave. Hablar por HTTP con una clave rompe la regla de "sin cuentas", así que es una decisión del dueño (ver "Decisiones abiertas").
- **Tamaño:** chico si se queda en comandos; medio si se agrega HTTP.

### 7. Herramientas para el modelo

- **En CodeCanvas:** el modelo puede leer archivos del proyecto, buscar en la web (con un SearXNG propio) y leer una página, mediante etiquetas de texto en la respuesta que la app interpreta. Funciona con cualquier modelo, tenga o no llamadas a herramientas.
- **En Code Canvas hoy:** `commit_ai.rs` lanza `claude -p` sin herramientas y `codex` en solo lectura.
- **Cómo encaja:** por el camino de los comandos esto casi no hay que hacerlo: Claude Code y Codex ya traen sus herramientas de lectura y búsqueda, y alcanza con permitirles las de solo lectura para el chat. El protocolo de etiquetas solo tendría sentido para modelos locales lanzados con `custom`.
- **Tamaño:** chico.

---

## Fragmentos

### 8. Regiones con nombre y color dentro de un archivo

- **En CodeCanvas:** arrastrás sobre unas líneas y las marcás como región: una barra de color al costado, un título, su propio chat y su propio borrador. La región sigue pegada a sus líneas cuando se edita arriba, abajo o adentro, porque todo cambio de texto pasa por una sola función que corre, agranda, achica o borra las regiones afectadas. Las regiones se pueden anidar.
- **En Code Canvas hoy:** no existe. Lo más cercano es el resaltado del cuerpo de un símbolo.
- **Cómo encaja:** no necesita tocar el layout, y es lo que habilita el chat por fragmento. La parte delicada es que una región sobreviva a todas las formas en que cambia un texto en Code Canvas, que son más que en CodeCanvas:
  - lo que se tipea (`Editor::splice`, `Editor::replace`);
  - deshacer y rehacer;
  - lo que escribe otro programa sobre un archivo con texto sin guardar (`Editor::take_in`);
  - lo que escribe otra persona en una sesión compartida (`Editor::apply_remote`);
  - la recarga desde disco y el formateo al guardar.
- **Tamaño:** medio.

### 9. Tarjetas flotantes

- **En CodeCanvas:** un rango de líneas se puede despegar del archivo como tarjeta suelta, copiar afuera, o se puede crear una tarjeta en blanco con doble clic en el lienzo vacío. Una tarjeta suelta se vuelve a unir arrastrándola sobre un archivo.
- **En Code Canvas hoy:** no existe, y el layout automático no tiene lugar para ellas.
- **Cómo encaja:** hace falta una capa nueva de tarjetas con posición propia, dibujada encima del layout y guardada en la sesión. Hay un antecedente parcial: los archivos de fuera de las carpetas abiertas ya tienen página propia (`Page::of_files`).
- **Tamaño:** grande. No conviene empezar por acá.

### 10. Tarjetas de referencia

- **En CodeCanvas:** una tarjeta que es espejo de unas líneas de un archivo. Editar la tarjeta edita el archivo; las líneas originales quedan atenuadas mientras la referencia existe.
- **En Code Canvas hoy:** no existe.
- **Cómo encaja:** depende de la idea 9. Sirve para poner lado a lado dos funciones que están lejos, algo que hoy se resuelve en parte con "mostrar solo los archivos conectados".
- **Tamaño:** grande.

### 11. Mover líneas arrastrando

- **En CodeCanvas:** con Shift y arrastre, las líneas seleccionadas se mueven dentro del archivo, a otro archivo o al lienzo. Mientras se arrastra, las líneas del destino se abren en vivo para mostrar dónde cae.
- **En Code Canvas hoy:** con el lápiz, arrastrar selecciona; no mueve.
- **Cómo encaja:** mover dentro del mismo archivo y entre archivos no depende de las tarjetas flotantes. La vista previa en vivo es la parte cara.
- **Tamaño:** medio.

---

## Lienzo y edición

### 12. Vistas divididas

- **En CodeCanvas:** el lienzo se parte en paneles en árbol, cada uno con su zoom y posición, sobre el mismo lienzo o cada uno con el suyo.
- **En Code Canvas hoy:** varias carpetas comparten un lienzo y una cámara. El renderizador ya sabe dibujar varios paneles en un mismo fotograma (ver "Several panes share one frame" en `docs/NOTES.md`).
- **Cómo encaja:** el caso útil es mirar dos lugares lejanos del mismo repositorio a la vez. Hoy cada página tiene una sola cámara, y habría que separarlas.
- **Tamaño:** medio a grande.

### 13. Acomodar tarjetas a mano

- **En CodeCanvas:** las tarjetas se mueven y se redimensionan, con selección múltiple por recuadro.
- **En Code Canvas hoy:** las tarjetas no se mueven; se pueden ocultar archivos y plegar carpetas.
- **Cómo encaja:** choca de frente con el layout automático, que es la identidad de la app. Una versión compatible sería una "mesa" aparte: una página con solo los archivos elegidos, que sí se acomodan a mano.
- **Tamaño:** grande.

### 14. Buscar con más opciones

- **En CodeCanvas:** expresiones regulares con grupos en el reemplazo (`$1`), palabra completa, mayúsculas exactas, conservar mayúsculas al reemplazar, y buscar solo dentro de la selección o de una región.
- **En Code Canvas hoy:** búsqueda de texto literal con "smart case" (`App::found_in`, `Repo::search_text`).
- **Cómo encaja:** son interruptores en la barra de búsqueda que ya existe. Ojo con dos cosas ya resueltas que no hay que romper: las columnas se cuentan en letras, no en bytes, y "reemplazar todo" es un solo paso de deshacer.
- **Tamaño:** chico.

### 15. Líneas largas que asoman

- **En CodeCanvas:** una línea más ancha que la tarjeta termina en un desvanecido, y al pasar el mouse el resto se dibuja saliendo por el costado derecho.
- **En Code Canvas hoy:** las líneas se cortan en 160 columnas (`MAX_COLS`). El ajuste de línea figura como pendiente en la hoja de ruta.
- **Cómo encaja:** es una alternativa más barata al ajuste de línea, y no cambia el alto de las tarjetas, así que no toca el layout.
- **Tamaño:** chico a medio.

### 16. Elegir el lenguaje de una tarjeta a mano

- **En CodeCanvas:** la etiqueta del lenguaje al lado del nombre se puede cambiar.
- **En Code Canvas hoy:** el coloreado es agnóstico al lenguaje (`canvas-core/src/syntax.rs`), así que hay poco para elegir.
- **Cómo encaja:** bajo valor mientras el coloreado siga siendo el propio. Cobraría sentido junto con las gramáticas de extensiones, que están pendientes en la hoja de ruta.
- **Tamaño:** chico, pero no prioritario.

---

## Integración

### 17. Instancia única

- **En CodeCanvas:** si la app ya está abierta para un proyecto, lanzarla de nuevo con un archivo lo muestra en la ventana que existe y el segundo proceso termina enseguida. La ventana abierta escucha en un puerto local cuyo número deja en un archivo.
- **En Code Canvas hoy:** `code-canvas [CARPETA ...]` abre siempre una ventana nueva y no acepta un archivo como destino.
- **Cómo encaja:** las dos mitades ya existen por separado. La app ya escucha en un puerto local para los asistentes (`agents.rs`, `agents.port`), y `App::show_path` ya es la única entrada por la que una ruta llega al lienzo. Falta el mensaje "mostrá esta ruta y esta línea" y el argumento de línea de comandos.
- **Tamaño:** chico.

### 18. Puente con otros editores y agentes

- **En CodeCanvas:** una extensión de Zed instala un atajo para abrir el proyecto en el lienzo y le da al agente de Zed una herramienta MCP `open_canvas`.
- **En Code Canvas hoy:** los asistentes avisan dónde están (una línea de JSON por UDP), pero no pueden pedirle nada al lienzo.
- **Cómo encaja:** con la idea 17 hecha, un servidor MCP mínimo que exponga "mostrá este archivo y esta línea" sirve para Claude Code, Zed o cualquier agente. Es el sentido inverso de lo que ya hay.
- **Tamaño:** chico, después de la idea 17.

---

# Parte 2: codex-canvas (DragosCP)

codex-canvas no es un lienzo de código. Es un fork de la app de escritorio de Codex (Electron) que al chat con el agente le agrega una vista previa de la app que se está construyendo. Ahí "canvas" es esa vista previa. Su idea central es el **modo Draft**: el agente arma primero la parte visible de una función en una copia del proyecto, la persona la prueba, y recién al aprobarla se convierte en cambios reales.

Por eso acá no hay funciones para copiar tal cual. Lo que sigue son las ideas que se pueden trasladar a un lienzo de código. Es una versión preliminar (0.1.0) y su README aclara que el modo Draft todavía se está definiendo.

### 19. La app corriendo, en el lienzo

- **En codex-canvas:** arranca el servidor local del proyecto, o se conecta a uno que ya corre, y muestra la app al lado del chat ("Live preview").
- **En Code Canvas hoy:** no existe. Pero ya hay una webview que se ubica sobre una tarjeta en cada fotograma y se pausa cuando la tarjeta queda chica o fuera de vista: es como se reproducen videos y PDF (`media::PLAYER`, `App::player_place`). Solo en Windows.
- **Cómo encaja:** una tarjeta más del lienzo que muestra `localhost:<puerto>`, al lado de la carpeta del frontend. Se ve el código y lo que el código produce en el mismo lugar. Lo que falta decidir es quién arranca el servidor: la app, o la persona en su terminal y la app solo se conecta.
- **Tamaño:** medio.

### 20. Del elemento en pantalla a su código

- **En codex-canvas:** la persona marca zonas de la vista previa y eso viaja como contexto al agente, para que los dos hablen de la misma parte de la pantalla. Cada marca lleva un identificador del elemento, su rectángulo, su texto, la ruta de la página y, cuando el elemento lo declara, el archivo fuente y el nombre del componente. Esos datos salen de atributos en el HTML (`data-canvas-id`, `data-canvas-source-file`, `data-canvas-component`).
- **En Code Canvas hoy:** no existe.
- **Cómo encaja:** con la idea 19 hecha, un clic sobre un elemento de la app lleva el lienzo al archivo y al componente que lo dibuja, y lo deja seleccionado con sus curvas. Es "¿dónde está esto en el código?" desde la pantalla en vez de desde un nombre. Sumado al chat (idea 1), el elemento marcado es contexto del pedido. El punto débil es que depende de que la app tenga esos atributos; habría que ver cómo los pone codex-canvas o qué exponen los frameworks en modo desarrollo.
- **Tamaño:** medio a grande. Depende de la idea 19.

### 21. Probar una función en una copia antes de escribirla en el proyecto

- **En codex-canvas:** un "draft workspace" es una copia de trabajo del proyecto, registrada en un archivo de la app, donde el agente hace los cambios. Tiene tres estados: activo, construido y descartado. Aplicarlo lleva los cambios al proyecto real y, si chocan con lo que hay, devuelve los conflictos sin tocar nada. Cada draft deja además una nota con el pedido dentro del proyecto (`.codex/drafts/<nombre>/brief.md`).
- **En Code Canvas hoy:** no existe. Lo cercano es ver un commit en el lienzo y "mostrar solo lo que cambió".
- **Cómo encaja:** Code Canvas ya pone varias carpetas en un mismo lienzo. La copia de trabajo puede ser una carpeta más, al lado de la real, mostrando solo los archivos que difieren, con el diff dentro de cada tarjeta. Es el borrador de la idea 3 llevado de un archivo a todo el proyecto. Con git, un worktree es la forma natural de hacer la copia. No se verificó cómo la hace codex-canvas (está en `electron/proposal-workspaces.cjs`, que no se leyó).
- **Tamaño:** grande.

### 22. Avisar cuando un borrador quedó desactualizado

- **En codex-canvas:** antes de aplicar una propuesta revisa que el contexto en que se creó siga siendo el actual: el mismo proyecto, la misma página, el mismo tamaño de pantalla, el mismo archivo fuente. Si algo cambió la marca como desactualizada, explica por qué y ofrece salidas: actualizar, rehacer sobre lo nuevo o archivar. La comparación es por rutas e identificadores, no por contenido.
- **En Code Canvas hoy:** no hay borradores todavía.
- **Cómo encaja:** va junto con la idea 3. Un borrador hecho sobre un texto que después cambió no se debería aplicar a ciegas. Code Canvas puede hacer algo mejor que avisar: ya sabe pasar una edición por encima de otra (`TextOp::transform`, lo que usa la sesión compartida y `Editor::take_in`), así que el borrador se puede rehacer solo sobre el texto nuevo y avisar únicamente cuando eso no se puede.
- **Tamaño:** chico, dentro de la idea 3.

### 23. Qué cambió cada asistente

- **En codex-canvas:** al terminar un turno del agente muestra un resumen de los archivos que tocó, con entrada a la revisión de cada uno.
- **En Code Canvas hoy:** el lienzo viaja a lo que un asistente cambia mientras lo cambia, y se puede seguir a un asistente. No queda un resumen de lo que hizo. "Un rastro de dónde estuvo" figura como pendiente en la hoja de ruta.
- **Cómo encaja:** `agents.rs` ya recibe cada edición de cada asistente. Guardando esa lista por asistente, un clic en su cara puede dejar en el lienzo solo los archivos que tocó, cada uno con su diff, igual que hoy un clic en un commit. Es revisar el trabajo de un asistente antes de que exista el commit.
- **Tamaño:** chico a medio.

### 24. Aprobar o rechazar desde el lienzo

- **En codex-canvas:** los pedidos de permiso del agente (correr un comando, editar un archivo) aparecen en la conversación con las opciones de aceptar o rechazar.
- **En Code Canvas hoy:** un asistente que espera se ve como tal (acción `wait`), pero no se le puede contestar desde el lienzo.
- **Cómo encaja:** mostrar el pedido junto a la cara del asistente, sobre el archivo que quiere tocar, y contestarlo ahí. Hay un obstáculo de diseño: los hooks de Claude Code se instalan a propósito de forma asíncrona para que una herramienta nunca espere a la app (`agents.rs`). Contestar un permiso exige lo contrario, así que habría que resolver qué pasa cuando la app no responde.
- **Tamaño:** medio, con riesgo.

### Lo que codex-canvas anuncia y todavía no tiene

- **Vocabulario compartido del proyecto:** nombres para pantallas, paneles, botones y flujos, para que los pedidos y los planes usen las palabras de quien construye. En Code Canvas, las regiones con nombre (idea 8) son ese vocabulario para el código.

---

## Decisiones abiertas para el dueño del proyecto

1. **Claves o comandos.** ¿El chat usa solo los programas instalados (`claude`, `codex`, comando propio), como los mensajes de commit, o se agregan proveedores por HTTP con clave? Lo segundo cambia la regla de "sin cuentas".
2. **Dónde viven regiones y chats.** En el directorio de datos de la app, como la sesión, o en un archivo dentro del repositorio para que viajen con el proyecto y se puedan compartir por git.
3. **Sesión compartida.** ¿Las regiones, los chats y los borradores los ven quienes se unieron? Hoy los asistentes de una sesión compartida todavía no se muestran a los invitados (pendiente en la hoja de ruta).
4. **Tarjetas libres.** Si se quiere la capa de tarjetas flotantes (ideas 4, 9, 10 y 13) o se deja el layout automático como única forma de ver el código.
5. **La app corriendo.** Si Code Canvas debe mostrar la app que se está construyendo (ideas 19 y 20) o quedarse en el código. Es un cambio de alcance, y hoy las webviews funcionan solo en Windows.

## Lo que no se verificó

- El código de CodeCanvas: todo lo dicho de esa app sale de su documentación.
- El código de codex-canvas: lo dicho sale de su README y de resúmenes automáticos de tres archivos, no de una lectura completa. En particular, no se sabe cómo copia el proyecto para un draft ni cómo llegan los atributos `data-canvas-*` al HTML.
- La segunda mitad de `docs/NOTES.md` de Code Canvas, salvo las secciones nombradas arriba.
- Las opciones exactas de `claude` y `codex` para streaming y para retomar una sesión.
- Si alguna de estas ideas ya está empezada en una rama que no sea la actual.
