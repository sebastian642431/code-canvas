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
- **Buscar**: explorador con casillas y filtro, `Ctrl+P` para ir a un archivo,
  `Ctrl+Shift+F` para buscar texto en todos los archivos.
- **Claude Code**: un panel de chat que usa el comando `claude` que ya tenés instalado
  y con sesión iniciada. No guarda ninguna API key. Solo lectura por defecto.
- **Es una app**: encabezado y paneles propios, barras de íconos a los costados, y
  recuerda carpetas, cámaras y disposición entre aperturas.

## Instalar y abrir

En Windows, para tu usuario (sin permisos de administrador):

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

## Extensiones

Qué se puede aprovechar de las extensiones de VS Code y qué no, y cómo va a ser el
sistema de extensiones propio: [docs/EXTENSIONS.md](docs/EXTENSIONS.md).

## Cómo aportar

Mirá [CONTRIBUTING.md](CONTRIBUTING.md) y la hoja de ruta en el [README](README.md#roadmap).

## Licencia

MIT o Apache-2.0, a elección. Incluye la fuente de íconos Lucide (ISC).
