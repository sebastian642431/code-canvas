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
  (cada una en su propio lienzo).
- **Símbolos**: clic en un nombre y se dibujan curvas a su definición, referencias,
  llamadas entrantes y salientes e implementaciones, con filtros por tipo y la opción
  de mostrar solo los archivos conectados. Funciona al instante con un índice propio
  por nombres (aproximado); los *language servers* darán resultados exactos.
- **Dependencias**: qué archivo importa a cuál, leído de los `import`. Al seleccionar
  un archivo o una carpeta salen curvas hacia lo que usa y desde lo que lo usa, con un
  panel que lista ambas cosas.
- **Buscar**: explorador con casillas y filtro, `Ctrl+P` para ir a un archivo,
  `Ctrl+Shift+F` para buscar texto en todos los archivos.
- **Archivos**: clic derecho en el explorador para crear, renombrar, borrar (a la
  papelera), copiar la ruta, mostrar en el explorador del sistema o abrir una
  terminal. Soltá un archivo en la ventana para copiarlo a la carpeta seleccionada.
- **Ayudas al escribir**: al tipear una ruta entre comillas se ofrecen los archivos
  y carpetas que hay ahí; en `package.json` y `Cargo.toml` cada dependencia dice si
  existe una versión más nueva.
- **Actualizaciones**: al abrir, si hay una versión más nueva la ofrece en un aviso y
  la instala sola. El panel del asistente sigue lo que va escribiendo hasta que subís.
- **Varias carpetas a la vez**: comparten un solo lienzo, una al lado de la otra y con
  el mismo zoom; `Ctrl+E` vuelve a una sola por vez.
- **Edición**: el código legible es un editor: clic y a escribir, con deshacer,
  copiar y pegar y `Ctrl+S`. Formatea al guardar (se apaga con `editor.formatOnSave`), los
  problemas se actualizan mientras escribís y los archivos cambiados por otro programa
  se recargan solos. `Ctrl+Shift+P` abre la paleta de comandos.
- **Extensiones de VS Code**: instala las mismas extensiones `.vsix` desde
  [Open VSX](https://open-vsx.org) y las ejecuta sin modificar. Los linters marcan los
  errores sobre el código y en un panel de problemas; `Shift+Alt+F` formatea con la
  configuración del proyecto. Las *webviews* (la interfaz propia de una extensión) se
  abren en el panel derecho: ahí se ve el panel oficial de Claude Code (por ahora solo
  en Windows). Las vistas de árbol todavía no se muestran.
- **Control de código**: un panel como el de VS Code (rama, mensaje y Commit, cambios
  preparados y sin preparar, sincronizar, últimos commits), las líneas cambiadas
  marcadas en el lienzo, "mostrar solo lo que cambió" y el historial (*timeline*) del
  archivo seleccionado. El botón Commit tiene push, sync y amend; el menú "…" tiene
  pull, push, fetch y stash; y la chispa del cuadro de mensaje le pide el mensaje a
  Claude Code, a Codex o a un comando tuyo.
- **Claude Code**: la extensión oficial, instalada desde el panel de extensiones; su
  panel se abre a la derecha de la ventana.
- **Es una app**: encabezado y paneles propios, barras de íconos a los costados, y
  recuerda carpetas, cámaras y disposición entre aperturas.

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
