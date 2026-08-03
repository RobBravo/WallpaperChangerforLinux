# Wallpaper Changer Linux

Rotador de fondos de pantalla para **KDE Plasma**, escrito en Rust. Corre como un
daemon en segundo plano que cambia el wallpaper del escritorio a intervalos
configurables, tomando las imágenes de una carpeta que elijas, más una interfaz
gráfica simple para configurarlo.

- Repositorio: https://github.com/RobBravo/WallpaperChangerforLinux
- Plataforma soportada: **KDE Plasma** únicamente (usa D-Bus y `org.kde.PlasmaShell`
  directamente). No funciona en GNOME, XFCE u otros escritorios.
- **Soporte multipantalla:** cada monitor conectado tiene su propia carpeta,
  intervalo y pausa, con rotación independiente. Con un solo monitor conectado
  se comporta igual que antes.
- Soporte para GNOME y XFCE está planeado — ver [`ROADMAP.md`](ROADMAP.md).

---

## Índice

- [Cómo funciona](#cómo-funciona)
- [Arquitectura del proyecto](#arquitectura-del-proyecto)
- [Instalación en KDE Plasma](#instalación-en-kde-plasma)
- [Uso diario](#uso-diario)
- [Archivos de configuración](#archivos-de-configuración)
- [Desinstalación](#desinstalación)
- [Solución de problemas](#solución-de-problemas)
- [Desarrollo](#desarrollo)

---

## Cómo funciona

El proyecto son **dos programas independientes** que nunca se hablan
directamente entre sí — solo se comunican leyendo y escribiendo los mismos
archivos en `~/.config/wallpaper-changer/`:

1. **`wallpaper-changer-daemon`** — un binario que corre en segundo plano
   (como servicio de usuario de systemd) desde que inicias sesión. Cada cierto
   intervalo, elige una imagen al azar de la carpeta configurada **de cada
   monitor conectado** y le pide a Plasma (vía D-Bus) que la use como fondo de
   ese monitor específico. Tiene su propio ícono en la bandeja del sistema con
   un menú rápido.

2. **`wallpaper-changer-gui`** — una ventana de configuración (hecha con
   [Slint](https://slint.dev)) con un desplegable para elegir **qué monitor**
   estás configurando, y para ese monitor: carpeta de imágenes, intervalo de
   rotación, pausar/reanudar, y forzar un cambio inmediato. Al cerrarla no
   termina el proceso: se minimiza a su propio ícono en la bandeja, y solo
   corre una instancia a la vez.

Ninguno de los dos necesita al otro para funcionar: el daemon rota los
wallpapers solo con lo que haya en `config.toml`, y la GUI simplemente edita
ese archivo. Si cierras la GUI o nunca la abres, el daemon sigue rotando con
la última configuración guardada.

### Multipantalla

Cada monitor conectado se identifica por un UUID estable que KDE ya le asigna
internamente (vía KScreen) — el mismo UUID persiste entre reinicios y aunque
cambies el monitor de puerto. `config.toml`/`state.toml` guardan una entrada
por monitor, cada una con su propia carpeta, intervalo, pausa, imagen actual y
próximo cambio — la rotación de un monitor no afecta a la de otro.

- **Monitor nuevo:** la primera vez que se detecta un monitor nunca antes
  visto, copia la carpeta/intervalo/pausa del monitor marcado como principal.
  Si no hay ninguno todavía, usa los valores por defecto (carpeta de Imágenes,
  30 minutos, sin pausar).
- **Monitor desconectado:** su configuración **no se borra** — sigue en
  `config.toml`/`state.toml` tal cual quedó, simplemente deja de rotar y
  desaparece del desplegable de la GUI hasta que se reconecte.
- **Detección de monitores conectados/desconectados:** el daemon revisa cada
  30 segundos; la GUI, cada 5 segundos mientras la ventana está abierta.
- El ícono de la bandeja del daemon ("Pausar/Reanudar", "Cambiar ahora") actúa
  sobre el **monitor principal** — para controlar un monitor secundario, usá
  el desplegable de la GUI.

### El ciclo de rotación (por cada monitor)

1. El daemon lee `config.toml` al arrancar (y cada vez que detecta que
   cambió).
2. Escanea la carpeta configurada de ese monitor (solo el nivel superior, sin
   subcarpetas) y arma una lista de imágenes soportadas: `.png`, `.jpg`,
   `.jpeg`, `.bmp` (sin importar mayúsculas/minúsculas).
3. Baraja esa lista y va sacando una imagen por vez, sin repetir ninguna
   hasta haberlas mostrado todas — al agotarlas, vuelve a barajar.
4. Cada vez que aplica una imagen, escribe el resultado en la entrada de ese
   monitor en `state.toml` (qué imagen quedó puesta y cuándo toca la
   próxima), que es lo que la GUI lee para mostrar la vista previa y la
   cuenta regresiva del monitor seleccionado.
5. Si editas la carpeta o el intervalo a mitad de sesión, el daemon lo
   detecta y lo aplica de inmediato, sin necesidad de reiniciarlo.

### Íconos en la bandeja del sistema

Verás **dos íconos distintos** en la bandeja mientras el daemon (y
opcionalmente la GUI) estén corriendo:

| Ícono | Pertenece a | Menú |
|---|---|---|
| Fondo de escritorio (tema del sistema) | `wallpaper-changer-daemon` | Pausar/Reanudar, Cambiar ahora, Abrir configuración, Salir |
| Marco de imagen (ícono propio, incluido) | `wallpaper-changer-gui` (solo si la abriste) | Mostrar/Ocultar ventana, Salir |

**"Salir" en el ícono del daemon detiene la rotación de fondos por completo**
hasta que reinicies el servicio — no lo confundas con "Salir" del ícono de la
GUI, que solo cierra la ventana de configuración.

---

## Arquitectura del proyecto

Workspace de Cargo con tres crates:

```
WallpaperChangerLinux/
├── core/     wallpaper-core       — modelos compartidos, sin UI ni daemon
│   └── src/
│       ├── config.rs    Config = Vec<MonitorConfig> (carpeta, intervalo, pausa
│       │                por monitor) + carga/guardado TOML + migración automática
│       │                del formato viejo (monitor único)
│       ├── state.rs     State = Vec<MonitorState> (imagen actual, próximo
│       │                cambio por monitor) + carga/guardado TOML
│       ├── monitors.rs  lista de monitores conectados + su UUID estable
│       │                (combina `kscreen-doctor --json` y
│       │                `~/.config/kwinoutputconfig.json`)
│       ├── scanner.rs   escaneo de la carpeta de imágenes
│       ├── queue.rs     cola de rotación "barajar y consumir sin repetir"
│       ├── backend.rs   trait WallpaperBackend (por si algún día se soporta otro DE)
│       └── kde_backend.rs   implementación para KDE Plasma vía D-Bus —
│                            aplica la imagen al monitor correcto por posición
├── daemon/   wallpaper-changer-daemon
│   └── src/
│       ├── main.rs      bucle principal (hilos + mpsc, sin runtime async) —
│       │                deadline y detección de conexión/desconexión por monitor
│       ├── engine.rs    motor de rotación — una cola independiente por monitor
│       ├── watcher.rs   observador de archivos (notify/inotify)
│       └── tray.rs      ícono de bandeja del daemon (ksni)
├── gui/      wallpaper-changer-gui
│   ├── src/
│   │   ├── main.rs      ventana + selector de monitor + integración con la bandeja
│   │   └── singleton.rs detección de instancia única (socket Unix)
│   └── ui/
│       ├── app-window.slint   ventana de configuración (con el ComboBox de monitor)
│       └── tray-icon.slint    ícono de bandeja propio (SystemTrayIcon nativo de Slint)
├── packaging/
│   └── wallpaper-changer-daemon.service   unidad de systemd (usuario)
├── install.sh
└── docs/superpowers/    specs y planes de diseño de cada feature (histórico)
```

**Por qué está separado así:** `wallpaper-core` no sabe nada de hilos, D-Bus
en vivo, ni interfaces gráficas — solo modela datos y lógica pura, así que es
fácil de testear. El daemon y la GUI son consumidores independientes de esa
librería que nunca importan código el uno del otro; toda su coordinación pasa
por los archivos compartidos, nunca por llamadas directas.

### Tecnologías usadas

| Propósito | Crate |
|---|---|
| Interfaz gráfica | [`slint`](https://crates.io/crates/slint) (incluye ícono de bandeja nativo) |
| Ícono de bandeja del daemon | [`ksni`](https://crates.io/crates/ksni) |
| D-Bus (aplicar el wallpaper en Plasma) | [`zbus`](https://crates.io/crates/zbus) |
| Observar cambios de archivos | [`notify`](https://crates.io/crates/notify) |
| Selector nativo de carpetas | [`rfd`](https://crates.io/crates/rfd) |
| Serialización de config/estado | `serde` + `toml` |

El daemon **no usa ningún runtime async** (nada de `tokio`) — es
deliberadamente simple: hilos de sistema operativo y canales `mpsc`.

---

## Instalación en KDE Plasma

### Requisitos previos

1. **KDE Plasma** (Wayland o X11).
2. **Rust** (edición 2021 o más nueva). Si no lo tenés instalado:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```
   Luego reiniciá la terminal o corré `source "$HOME/.cargo/env"`.
3. **`git`** para clonar el repositorio.
4. Las librerías de desarrollo típicas de un entorno gráfico KDE Plasma
   (Wayland/X11, fontconfig, etc.) — si ya podés compilar y correr
   aplicaciones gráficas en tu sistema, ya las tenés. En Fedora, por ejemplo,
   el grupo `@development-tools` junto con un entorno Plasma completo alcanza.

### Pasos

1. **Cloná el repositorio:**
   ```bash
   git clone https://github.com/RobBravo/WallpaperChangerforLinux.git
   cd WallpaperChangerforLinux
   ```

2. **Corré el script de instalación:**
   ```bash
   ./install.sh
   ```
   Esto hace, en orden:
   - Compila los tres binarios en modo release (`cargo build --release --locked`
     — la primera vez tarda varios minutos, ya que Slint trae bastantes
     dependencias).
   - Detiene el daemon si ya estaba corriendo (para poder reemplazar el
     binario sin errores).
   - Instala `wallpaper-changer-daemon` y `wallpaper-changer-gui` en
     `~/.local/bin/`.
   - Instala la unidad de systemd de usuario en
     `~/.config/systemd/user/wallpaper-changer-daemon.service`.
   - Habilita y arranca el servicio (`systemctl --user enable --now`).

3. **Verificá que el daemon quedó corriendo:**
   ```bash
   systemctl --user status wallpaper-changer-daemon
   ```
   Deberías ver `Active: active (running)` y un nuevo ícono en la bandeja del
   sistema.

4. **Abrí la ventana de configuración** desde el ícono de la bandeja del
   daemon ("Abrir configuración") o ejecutando directamente:
   ```bash
   ~/.local/bin/wallpaper-changer-gui
   ```

5. **Configurá tu carpeta de fondos:** si tenés más de un monitor, elegí cuál
   estás configurando en el desplegable de arriba. Hacé clic en "Elegir…",
   seleccioná la carpeta con tus imágenes, ajustá el intervalo y guardá.
   Repetí para cada monitor que quieras configurar — cada uno tiene su propia
   carpeta e intervalo, independientes entre sí.

El servicio queda habilitado para arrancar automáticamente en cada inicio de
sesión — no hace falta abrir la GUI de nuevo salvo que quieras cambiar la
configuración.

---

## Uso diario

- **Cambiar de carpeta o intervalo:** abrí la GUI, elegí el monitor en el
  desplegable si tenés más de uno, ajustá los campos, "Guardar".
- **Pausar la rotación de un monitor sin perder su configuración:** botón
  "Pausar" en la GUI (afecta solo al monitor seleccionado en el desplegable),
  o "Pausar/Reanudar" en la bandeja del daemon (afecta al monitor **principal**
  — usá la GUI para pausar un monitor secundario).
- **Forzar un cambio inmediato:** botón "Cambiar ahora" en la GUI (solo para
  el monitor seleccionado), o la misma opción en la bandeja del daemon (solo
  para el monitor principal).
- **Minimizar la GUI:** cerrala con el botón de la ventana — se oculta a su
  propio ícono en la bandeja en vez de cerrarse. Para volver a abrirla, hacé
  clic en ese ícono y elegí "Mostrar/Ocultar ventana", o volvé a lanzar
  `wallpaper-changer-gui` (si ya hay una instancia abierta, simplemente la
  trae al frente en vez de abrir una nueva).
- **Cerrar la GUI de verdad:** "Salir" en el menú de su propio ícono de
  bandeja.

> **Nota sobre Wayland:** si la ventana de la GUI queda minimizada por la
> barra de tareas (no solo oculta con el botón de cerrar) y usás una sesión
> Wayland, restaurarla desde la bandeja puede no des-minimizarla — es una
> limitación conocida de Slint/winit sobre el protocolo Wayland, no un error
> de la aplicación. Cerrala y volvé a abrirla si esto pasa.

---

## Archivos de configuración

Todo vive bajo `~/.config/wallpaper-changer/` (se crea solo la primera vez
que corre cualquiera de los dos binarios):

| Archivo | Quién lo escribe | Contenido |
|---|---|---|
| `config.toml` | GUI, o vos a mano | una entrada por monitor: carpeta, intervalo, si está pausado |
| `state.toml` | el daemon | una entrada por monitor: imagen actual, próximo cambio (solo lectura para la GUI) |
| `change_now_request` | GUI o bandeja | UUID del monitor a cambiar de inmediato |
| `gui.sock` | la GUI | socket para detectar que ya hay una instancia abierta |

`config.toml` es editable a mano si preferís no usar la GUI — una tabla
`[[monitors]]` por cada monitor, identificado por su UUID (lo podés obtener
con `kscreen-doctor -o` o mirando la entrada correspondiente en
`~/.config/kwinoutputconfig.json`):

```toml
[[monitors]]
uuid = "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
folder = "/home/tu_usuario/Imágenes/Wallpapers"
interval_value = 30
interval_unit = "minutes"   # "minutes" | "hours" | "days"
paused = false

[[monitors]]
uuid = "otro-uuid-de-otro-monitor"
folder = "/home/tu_usuario/Imágenes/OtroFondo"
interval_value = 1
interval_unit = "hours"
paused = false
```

Si venís de una versión anterior a esta (formato de un solo monitor, sin
`[[monitors]]`), el daemon migra el archivo automáticamente la primera vez
que arranca — tu carpeta e intervalo se conservan, asignados al monitor que
esté marcado como principal en ese momento.

El daemon detecta los cambios en este archivo automáticamente (no hace falta
reiniciarlo) — si el archivo queda con un error de sintaxis, el daemon
conserva la última configuración válida en memoria y no se cae.

---

## Desinstalación

```bash
systemctl --user disable --now wallpaper-changer-daemon
rm ~/.config/systemd/user/wallpaper-changer-daemon.service
rm ~/.local/bin/wallpaper-changer-daemon ~/.local/bin/wallpaper-changer-gui
rm -rf ~/.config/wallpaper-changer
systemctl --user daemon-reload
```

---

## Solución de problemas

**El ícono de la bandeja no aparece:** confirmá que tu barra de tareas de
Plasma tiene el widget "Área de notificaciones del sistema" con los íconos
ocultos configurados para mostrar esta app, o simplemente que no esté
oculta ahí.

**El fondo no rota:** revisá los logs del daemon:
```bash
journalctl --user -u wallpaper-changer-daemon -f
```
Causas comunes: la carpeta configurada no tiene imágenes con extensión
soportada (`png`, `jpg`, `jpeg`, `bmp`) en su nivel superior, o el daemon
está pausado.

**Quiero ver si el binario instalado corresponde al código actual:**
```bash
systemctl --user restart wallpaper-changer-daemon
```
(o volvé a correr `./install.sh` después de un `git pull`).

**Conecté un monitor nuevo y no aparece en el desplegable de la GUI:**
esperá unos segundos — la GUI vuelve a revisar los monitores conectados cada
5 segundos mientras la ventana está abierta (el daemon, cada 30 segundos). Si
seguís sin verlo, confirmá que `kscreen-doctor --json` lo lista como
`connected` y `enabled` (un monitor con la tapa cerrada o desactivado en
Configuración del Sistema no cuenta, aunque siga conectado físicamente).

**El fondo se aplicó al monitor equivocado:** confirmá que `kscreen-doctor
--json` reporta la posición (`pos`) correcta de cada monitor — la app deduce
a cuál `Desktop` de Plasma corresponde cada monitor por su posición relativa
(de arriba hacia abajo, de izquierda a derecha), no por nombre. Si tenés
varios monitores con posiciones fuera de lo común (superpuestos, rotados), es
el escenario menos probado de esta función.

---

## Desarrollo

```bash
cargo build --workspace          # compilar todo
cargo test --workspace           # correr toda la suite de tests
cargo run -p wallpaper-changer-daemon   # correr el daemon en primer plano (sin systemd)
cargo run -p wallpaper-changer-gui      # correr la GUI directamente
```

El historial de diseño de cada feature (specs y planes de implementación)
vive en `docs/superpowers/` — útil si querés entender el razonamiento
detrás de alguna decisión de arquitectura antes de tocar el código.
