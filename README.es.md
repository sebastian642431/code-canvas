# Code Canvas

**Mirá un código entero de una sola vez.** Code Canvas dibuja todas las carpetas y
archivos de un repositorio en un lienzo infinito renderizado por GPU. Alejate para ver
la forma del proyecto completo; acercate hasta leer el código real, con colores, en el
mismo lugar. Tocá una función y aparecen curvas hacia donde se define, se referencia y
se llama.

Hecho en Rust con [wgpu](https://wgpu.rs), [winit](https://github.com/rust-windowing/winit)
y [egui](https://github.com/emilk/egui). Pensado para mantenerse rápido con repositorios
muy grandes. Código abierto: cualquiera puede aportar.

> Read in English: [README.md](README.md)

## Qué hace

- **Lienzo**: carpetas como cajas anidadas, archivos como tarjetas; cinco niveles de
  detalle; carpetas colapsables, archivos ocultables, varias carpetas abiertas a la vez
  en el mismo lienzo.
- **Símbolos**: clic en un nombre y se dibujan curvas a su definición, referencias,
  llamadas entrantes y salientes e implementaciones, con filtros por tipo y la opción
  de mostrar solo los archivos conectados. Funciona al instante con un índice propio
  por nombres (aproximado); con la extensión del lenguaje instalada (rust-analyzer,
  TypeScript…) los usos del nombre elegido pasan a ser los exactos.
- **Dependencias**: qué archivo importa a cuál, leído de los `import`. Al seleccionar
  un archivo o una carpeta salen curvas hacia lo que usa y desde lo que lo usa, con un
  panel que lista ambas cosas.
- **Buscar**: explorador con casillas y filtro, `Ctrl+P` para ir a un archivo,
  `Ctrl+Shift+F` para buscar texto en todos los archivos.
- **Archivos**: clic derecho en el explorador para crear, renombrar, borrar (a la
  papelera), copiar la ruta, mostrar en el explorador del sistema o abrir una
  terminal. Soltá un archivo en la ventana para copiarlo a la carpeta seleccionada;
  si es un video, una imagen o un PDF se abre en el visor al terminar. Lo que git
  ignora no va al lienzo, pero el explorador lo lista atenuado.
- **Markdown como se lee**: *Preview*, al lado del nombre de un `.md` en el lienzo (o
  en el menú del explorador), lo muestra renderizado como en GitHub, en el visor, y
  sigue lo que vas escribiendo. Los enlaces a otros archivos los abren; los de la web
  van a tu navegador.
- **Ayudas al escribir**: al tipear una ruta entre comillas se ofrecen los archivos
  y carpetas que hay ahí; en `package.json` y `Cargo.toml` cada dependencia dice si
  existe una versión más nueva.
- **Actualizaciones**: al abrir, si hay una versión más nueva la ofrece en un aviso y
  la instala sola. El panel del asistente sigue lo que va escribiendo hasta que subís.
- **Temas de VS Code**: instalá un tema de colores como extensión (Dracula, GitHub
  Theme, Tokyo Night…) y elegilo en Ajustes, Apariencia: la ventana, el lienzo y el
  código toman sus colores, también los claros.
- **Ajustes** (el engranaje, o `Ctrl+,`): se abren en una ventana sobre la app, con
  versión y actualizaciones, tema, íconos, cada color, cada atajo y un buscador que
  recorre todo. La ventana abre donde y como la dejaste (maximizada si lo estaba) y
  nunca más grande que la pantalla.
- **Varias carpetas a la vez**: comparten un solo lienzo, una al lado de la otra y con
  el mismo zoom; `Ctrl+E` vuelve a una sola por vez.
- **Mirar, editar o dibujar**: la barra de abajo tiene tres herramientas. Con la **mano**
  (`H`) solo se recorre el lienzo: nada se modifica por accidente. Con el **cursor** (`E`)
  el código legible es un editor: clic y a escribir, con deshacer, copiar y pegar y
  `Ctrl+S`. Con el **lápiz** (`D`) se hacen garabatos sobre el lienzo para señalar cosas
  mientras lo hablás con alguien: seis colores, tres anchos, una goma que borra solo
  la parte por la que pasa y una papelera que lo borra todo.
- **Lo que escribís se ve al instante**, antes de guardar: un punto en el nombre del
  archivo, sus líneas cambiadas coloreadas y el archivo en Control de código fuente.
  Formatea al guardar (se apaga con `editor.formatOnSave`) y los problemas se actualizan
  mientras escribís (Rust con rust-analyzer, que la app ofrece instalar).
- **Seguir a la IA**: cuando un asistente u otro programa cambia un archivo, el lienzo
  viaja a las líneas que cambiaron y las ilumina. Se apaga en la configuración.
- Las tarjetas (Source Control, Search, el chat de un asistente…) se arrastran por su
  título a cualquier lugar del lienzo y se agrandan o achican por sus bordes y la
  esquina; doble clic en el título las vuelve a acomodar.
- **Buscar y reemplazar** en un archivo con `Ctrl+F`; con el cursor, arrastrar selecciona.
- **Imágenes y videos dentro del lienzo**: la imagen se dibuja en su tarjeta, y un video,
  un audio o un PDF se reproduce dentro de su tarjeta al tocarlo.
- **Cualquier ruta se abre en el lienzo**: la que nombra un asistente, una definición, un
  enlace. Si el archivo no está en una carpeta abierta, aparece como una tarjeta más.
- **Atrás y adelante**: las flechas de la barra, `Ctrl+Z` / `Ctrl+Y` o `Alt+←` / `Alt+→`
  vuelven a donde estaba el lienzo antes del último salto.
- **Ver qué cambió dentro de cada archivo** (Control de código fuente): lo que se quitó
  en rojo arriba de lo que lo reemplazó, o lo que era y lo que es **lado a lado** en dos
  columnas de la misma tarjeta, con las líneas sin cambios plegadas, para que un archivo
  largo sea solo sus cambios.
  `Ctrl+Shift+P` abre la paleta de comandos.
- **Extensiones de VS Code**: instala las mismas extensiones `.vsix` desde
  [Open VSX](https://open-vsx.org) y las ejecuta sin modificar. Los linters marcan los
  errores sobre el código y en un panel de problemas; `Shift+Alt+F` formatea con la
  configuración del proyecto. Las *webviews* (la interfaz propia de una extensión) se
  abren como tarjeta sobre el lienzo: ahí se ve el panel oficial de Claude Code (por
  ahora solo en Windows). Las vistas de árbol todavía no se muestran.
- **Autocompletado**: con la extensión del lenguaje instalada (rust-analyzer,
  TypeScript y similares), mientras se escribe aparecen los miembros después de un
  punto y los nombres que encajan, con su tipo. Enter o Tab lo toma y agrega el
  `import` / `use` que haga falta.
- **Ver un commit en el lienzo**: un clic en un commit (en el *Graph* o en el *timeline*
  de un archivo) deja en el lienzo solo los archivos que tocó, cada uno mostrando qué
  le cambió. La cruz de la barra de arriba, o el mismo clic otra vez, vuelve a los
  archivos como están.
- **Clic derecho sobre una línea cambiada**: prepara (*stage*), saca del *stage* o
  descarta solo ese cambio; el mismo menú prepara el archivo entero o copia su ruta.
- **IAs en el lienzo**: cada asistente que trabaja sobre tus archivos aparece en el
  lienzo con su propio avatar y nombre: qué líneas está leyendo, qué busca, qué
  acaba de cambiar. Una sesión de Claude Code es un avatar y cada agente que lanza
  es otro. Clic en su cara (arriba) para **seguirlo**. Se activa en Ajustes,
  *Assistants on the canvas*, *Connect Claude Code*.
- **Workspace**: cada carpeta que abrís queda en la lista *Workspace* del panel de
  archivos. Arrastrás una al lienzo (o le hacés clic) y aparece junto a las demás;
  la cruz, o el clic derecho en el lienzo, la saca sin olvidarla.
- **Compartir la sesión** (el ícono de arriba a la derecha): la app te da una
  invitación para copiar; quien la pega en su Code Canvas ve tus carpetas abiertas
  en su propio lienzo, y lo que cualquiera escribe aparece para todos mientras se
  escribe. Cada invitado entra como lector o como escritor (lo cambiás cuando
  quieras), se ve el puntero, el cursor y la vista de cada persona con su nombre y
  color, y podés **seguir** a alguien o traer a todos a lo que estás mirando. A
  quien entra no se le limita nada: sus extensiones y asistentes trabajan sobre tus
  carpetas, y los archivos que crea, cambia o borra (si lo dejás editar) se crean,
  cambian o borran en tu computadora; por eso sirve para trabajar en tu otra PC, y
  por eso la app avisa antes de compartir y antes de unirse: solo con alguien de
  confianza. Se corta con un botón y sola al cerrar la app. En la misma red funciona directo;
  desde afuera hace falta una VPN (Tailscale, ZeroTier) o abrir el puerto.
- **Control de código**: un panel como el de VS Code (rama, mensaje y Commit, cambios
  preparados y sin preparar, sincronizar, últimos commits), las líneas cambiadas
  marcadas en el lienzo, "mostrar solo lo que cambió" y el historial (*timeline*) del
  archivo seleccionado. El botón Commit tiene push, sync y amend; el menú "…" tiene
  pull, push, fetch y stash; y la chispa del cuadro de mensaje le pide el mensaje a
  Claude Code, a Codex o a un comando tuyo.
- **Claude Code**: la extensión oficial, instalada desde el panel de extensiones; su
  panel se abre a la derecha de la ventana.
- **Clic derecho en cualquier parte del lienzo**: menú según lo que haya debajo
  (archivo: ocultar, renombrar, borrar, copiar ruta; carpeta: plegar; lienzo: ver
  todo, comandos). *Hide the interface* (o `Ctrl+\`) deja solo el lienzo.
- Al seleccionar un archivo en el lienzo, el explorador abre sus carpetas y lo marca.
- **Consola**: `` Ctrl+` `` (o su ícono) abre bash en la carpeta abierta, como una
  tarjeta sobre el lienzo. En Windows usa el bash de Git para Windows; si no está
  instalado, la tarjeta ofrece instalarlo.
- **Los archivos quedan pegados a la izquierda** (`Ctrl+B` o el botón de arriba los
  oculta); **todo lo demás se abre como tarjeta flotante sobre el lienzo**: búsqueda,
  control de código, símbolos, un asistente. Abrís las que quieras, una al lado de la
  otra, las ensanchás arrastrando el borde y cada una se cierra con su cruz. El lienzo
  sigue por detrás.
- **Es una app**: encabezado propio, barras de íconos a los costados, y recuerda
  carpetas, cámaras, tarjetas abiertas y disposición entre aperturas. Los íconos de
  lo propio de la app arrancan en la barra izquierda y los de las extensiones en la
  derecha; cualquier ícono se puede arrastrar a otro lugar o a la otra barra (o con
  clic derecho) y queda donde lo dejaste.

## Instalar y abrir

**Descargar**: en la [página de versiones](https://github.com/sebastian642431/code-canvas/releases)
está `code-canvas.exe` para Windows. Es un solo archivo: se ejecuta, sin instalador.
Windows puede avisar que es de un editor desconocido (todavía no está firmado): "Más
información" → "Ejecutar de todas formas". Las extensiones necesitan además
[Node.js](https://nodejs.org).

Desde el código, en Windows, para tu usuario (sin permisos de administrador):

```powershell
powershell -ExecutionPolicy Bypass -File scripts\install.ps1
```

Compila, copia el ejecutable a `%LOCALAPPDATA%\Programs\Code Canvas` y crea accesos
directos en el Menú Inicio y el Escritorio. Con `-ContextMenu` agrega "Open with Code
Canvas" al clic derecho de las carpetas. `scripts\uninstall.ps1` lo quita todo.

Desde el código, en cualquier plataforma:

```sh
cargo run --release -- ruta/al/frontend ruta/al/backend
```

Sin argumentos restaura la sesión anterior. Todos los atajos están en el
[README en inglés](README.md#install-and-run) y dentro de la app (ícono de ayuda).

## Rendimiento

Prueba de estrés con **33.500 archivos / 14,1 M de líneas / 2,27 M de símbolos**: abre
en ~2,8 s sin bloquear la ventana y se mueve a 240 fps (0,7 ms de CPU por fotograma)
en cualquier nivel de zoom. Cómo se logra: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

Con **Chromium** (514.507 archivos / 63,7 M de líneas): abre en ~19 s con los archivos
ya en la caché del disco y dibuja el repositorio entero en ~5 ms por fotograma. Usa
~4 GB de memoria porque hoy guarda el texto de todos los archivos; leerlo a demanda es
lo próximo.

## Extensiones

Cómo se ejecutan las extensiones de VS Code, cuáles están probadas y qué falta
(*webviews*, vistas de árbol): [docs/EXTENSIONS.md](docs/EXTENSIONS.md).

## Cómo aportar

Mirá [CONTRIBUTING.md](CONTRIBUTING.md) y la hoja de ruta en el [README](README.md#roadmap).

## Licencia

MIT o Apache-2.0, a elección. Incluye las fuentes de íconos Lucide (ISC) y Codicons,
los íconos de VS Code (CC BY 4.0).
