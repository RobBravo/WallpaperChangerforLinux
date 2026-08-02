# Wallpaper Changer Linux

Rotador de fondos de pantalla para **KDE Plasma**, escrito en Rust. Corre como un
daemon en segundo plano que cambia el wallpaper del escritorio a intervalos
configurables, tomando las imágenes de una carpeta que elijas, más una interfaz
gráfica simple para configurarlo.

- Repositorio: https://github.com/RobBravo/WallpaperChangerforLinux
- Plataforma soportada: **KDE Plasma** únicamente (usa D-Bus y `org.kde.PlasmaShell`
  directamente). No funciona en GNOME, XFCE u otros escritorios.
- Monitor único: no hay lógica de multi-monitor en esta versión.

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
   intervalo, elige una imagen al azar de la carpeta configurada y le pide a
   Plasma (vía D-Bus) que la use como fondo de pantalla. Tiene su propio ícono
   en la bandeja del sistema con un menú rápido.

2. **`wallpaper-changer-gui`** — una ventana de configuración (hecha con
   [Slint](https://slint.dev)) para elegir la carpeta de imágenes, el
   intervalo de rotación, pausar/reanudar, y forzar un cambio inmediato. Al
   cerrarla no termina el proceso: se minimiza a su propio ícono en la
   bandeja, y solo corre una instancia a la vez.

Ninguno de los dos necesita al otro para funcionar: el daemon rota el
wallpaper solo con lo que haya en `config.toml`, y la GUI simplemente edita
ese archivo. Si cierras la GUI o nunca la abres, el daemon sigue rotando con
la última configuración guardada.

### El ciclo de rotación

1. El daemon lee `config.toml` al arrancar (y cada vez que detecta que
   cambió).
2. Escanea la carpeta configurada (solo el nivel superior, sin subcarpetas) y
   arma una lista de imágenes soportadas: `.png`, `.jpg`, `.jpeg`, `.bmp`
   (sin importar mayúsculas/minúsculas).
3. Baraja esa lista y va sacando una imagen por vez, sin repetir ninguna
   hasta haberlas mostrado todas — al agotarlas, vuelve a barajar.
4. Cada vez que aplica una imagen, escribe el resultado en `state.toml` (qué
   imagen quedó puesta y cuándo toca la próxima), que es lo que la GUI lee
   para mostrar la vista previa y la cuenta regresiva.
5. Si editas la carpeta a mitad de sesión (agregás o borrás imágenes), el
   daemon lo detecta en la siguiente rotación sin necesidad de reiniciarlo.

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
│       ├── config.rs    Config (carpeta, intervalo, pausa) + carga/guardado TOML
│       ├── state.rs     State (imagen actual, próximo cambio) + carga/guardado TOML
│       ├── scanner.rs   escaneo de la carpeta de imágenes
│       ├── queue.rs     cola de rotación "barajar y consumir sin repetir"
│       ├── backend.rs   trait WallpaperBackend (por si algún día se soporta otro DE)
│       └── kde_backend.rs   implementación para KDE Plasma vía D-Bus
├── daemon/   wallpaper-changer-daemon
│   └── src/
│       ├── main.rs      bucle principal (hilos + mpsc, sin runtime async)
│       ├── engine.rs    motor de rotación
│       ├── watcher.rs   observador de archivos (notify/inotify)
│       └── tray.rs      ícono de bandeja del daemon (ksni)
├── gui/      wallpaper-changer-gui
│   ├── src/
│   │   ├── main.rs      ventana + integración con la bandeja
│   │   └── singleton.rs detección de instancia única (socket Unix)
│   └── ui/
│       ├── app-window.slint   ventana de configuración
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

5. **Configurá tu carpeta de fondos:** hacé clic en "Elegir…", seleccioná la
   carpeta con tus imágenes, ajustá el intervalo y guardá.

El servicio queda habilitado para arrancar automáticamente en cada inicio de
sesión — no hace falta abrir la GUI de nuevo salvo que quieras cambiar la
configuración.

---

## Uso diario

- **Cambiar de carpeta o intervalo:** abrí la GUI, ajustá los campos, "Guardar".
- **Pausar la rotación sin perder la configuración:** botón "Pausar" en la GUI
  o "Pausar/Reanudar" en la bandeja del daemon (ambos hacen lo mismo).
- **Forzar un cambio inmediato:** botón "Cambiar ahora" en la GUI, o la misma
  opción en la bandeja del daemon.
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
| `config.toml` | GUI, o vos a mano | carpeta, intervalo, si está pausado |
| `state.toml` | el daemon | imagen actual, próximo cambio (solo lectura para la GUI) |
| `change_now_request` | GUI o bandeja | señal para forzar un cambio inmediato |
| `gui.sock` | la GUI | socket para detectar que ya hay una instancia abierta |

`config.toml` es editable a mano si preferís no usar la GUI:

```toml
folder = "/home/tu_usuario/Imágenes/Wallpapers"
interval_value = 30
interval_unit = "minutes"   # "minutes" | "hours" | "days"
paused = false
```

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
