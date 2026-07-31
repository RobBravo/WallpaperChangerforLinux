# Wallpaper Changer Linux — Design

## Resumen

Aplicación en Rust para Linux que rota automáticamente el fondo de pantalla de un solo
monitor, tomando imágenes de una carpeta elegida por el usuario, en un intervalo
configurable (minutos/horas/días). Corre en segundo plano como servicio de usuario que
se inicia con el sistema, con una interfaz gráfica simple para configurarla. Primera
versión enfocada en KDE Plasma, con arquitectura extensible a otros entornos de
escritorio.

## Alcance

**Incluido en v1:**
- Selección de carpeta de fondos (solo nivel superior, sin subcarpetas).
- Intervalo configurable en minutos, horas o días.
- Rotación aleatoria sin repetir hasta agotar la carpeta.
- Ejecución en segundo plano como servicio systemd de usuario, con autoarranque.
- Icono en la bandeja del sistema con acciones rápidas (pausar, cambiar ahora, abrir
  configuración, salir).
- GUI simple para configurar carpeta e intervalo, con preview del fondo activo y
  cuenta regresiva del próximo cambio.
- Soporte para KDE Plasma (vía D-Bus).

**Explícitamente fuera de alcance en v1:**
- Multi-monitor (se maneja un solo monitor / fondo único).
- Backends para GNOME, XFCE u otros entornos (la arquitectura queda lista para
  añadirlos después, pero no se implementan ahora).
- Notificaciones de escritorio al cambiar de fondo.
- Empaquetado (rpm, flatpak, AppImage, etc.).
- Persistencia del conteo del intervalo entre reinicios (siempre reinicia desde cero).
- Formatos de imagen más allá de png/jpg/jpeg/bmp.

## Arquitectura

Workspace de Rust con tres crates:

1. **`core`** (librería compartida): modelo de configuración y de estado, trait
   `WallpaperBackend`, implementación del backend de KDE Plasma, escáner de carpeta de
   imágenes, lógica de selección aleatoria sin repetición.
2. **`daemon`** (binario, sin ventana): proceso que corre siempre en segundo plano.
   Aplica el fondo de pantalla, gestiona el temporizador, observa los archivos de
   configuración/comandos, y expone el icono en la bandeja del sistema.
3. **`gui`** (binario, Slint): ventana de configuración que se abre bajo demanda desde
   el menú de la bandeja o el lanzador de aplicaciones. No autoarranca ni queda
   corriendo en segundo plano — se cierra normalmente al terminar.

Elegimos esta separación (en vez de un solo proceso) para que la rotación de fondos siga
funcionando de forma confiable e independiente aunque la ventana de configuración esté
cerrada, con responsabilidades claramente separadas entre "motor" (daemon) y
"configurador" (GUI).

## Datos y comunicación entre procesos

Todo vive en `~/.config/wallpaper-changer/`:

- **`config.toml`** — único archivo que edita la GUI. Contiene:
  - `folder: PathBuf` — carpeta de fondos.
  - `interval_value: u64` y `interval_unit: "minutes" | "hours" | "days"`.
  - `paused: bool`.

  El daemon observa este archivo con el crate `notify`. Al detectar un cambio, recarga
  la configuración: si cambió la carpeta o el intervalo, reinicia el temporizador desde
  cero con el nuevo valor; si cambió `paused`, pausa o reanuda la rotación.

- **`change_now_request`** — archivo señal, vacío o con un timestamp, que la GUI
  crea/actualiza cuando el usuario pulsa "Cambiar ahora". El daemon lo detecta vía el
  mismo watch, aplica un fondo nuevo de inmediato y reinicia el temporizador del
  intervalo (mismo comportamiento que un reinicio del daemon: se arranca un conteo
  nuevo completo).

- **`state.toml`** — solo el daemon escribe aquí, en cada cambio de fondo. Contiene la
  ruta del fondo activo y la hora del próximo cambio (RFC3339). La GUI lo lee al
  abrirse y lo vuelve a leer cada segundo mientras la ventana está abierta, para
  refrescar el preview y la cuenta regresiva. No requiere watch: es una lectura local
  barata hecha por polling simple.

Los botones "Pausar/Reanudar" y "Cambiar ahora" de la GUI actúan de inmediato (escriben
su archivo correspondiente apenas se pulsan), independientemente del botón "Guardar",
que persiste los cambios de carpeta e intervalo.

**Excepción de D-Bus:** el IPC interno de la aplicación (GUI ↔ daemon) es por archivo +
watch, tal como se decidió. Sin embargo, hablar con **Plasma** para efectivamente
cambiar el fondo de pantalla requiere D-Bus, porque es la única API que expone KDE para
hacerlo por script (`org.kde.PlasmaShell.evaluateScript`). Se usará el crate `zbus`
(cliente D-Bus puro en Rust) en vez de invocar el binario `qdbus` como subproceso, para
no depender de un binario externo del sistema.

## Comportamiento del daemon

- Al iniciar: cachea el estado y, si no está pausado, escanea la carpeta (solo nivel
  superior; extensiones `png`, `jpg`, `jpeg`, `bmp`), elige un fondo aleatorio, lo
  aplica vía el backend de KDE, escribe `state.toml`, y arranca el temporizador con el
  intervalo completo configurado.
- Selección aleatoria: se baraja la lista de imágenes de la carpeta y se consume como
  cola; no se repite ningún fondo hasta agotar la carpeta completa, momento en el que
  se vuelve a barajar.
- Icono de bandeja: usando el crate `ksni` (protocolo StatusNotifierItem, nativo de
  Plasma). Menú: **Pausar/Reanudar**, **Cambiar ahora**, **Abrir configuración**,
  **Salir**.
- Manejo de errores: si la carpeta configurada queda vacía, no existe, o el backend
  falla al aplicar el fondo, el daemon no se cae — mantiene el fondo actual y registra
  el error. Como corre como servicio systemd de usuario, estos errores quedan
  disponibles vía `journalctl --user -u wallpaper-changer-daemon`.

## GUI

Ventana de una sola columna (layout validado con el usuario), de arriba hacia abajo:

1. Preview del fondo activo (leído de `state.toml`).
2. Selector de carpeta de fondos (campo de texto + botón "Elegir…" que abre un diálogo
   nativo de selección de carpeta).
3. Campo de intervalo: número + dropdown de unidad (minutos/horas/días).
4. Texto de cuenta regresiva hasta el próximo cambio (calculado desde
   `next_change_at` en `state.toml`, refrescado cada segundo).
5. Botones: **Pausar/Reanudar** (inmediato), **Cambiar ahora** (inmediato), **Guardar**
   (persiste carpeta + intervalo en `config.toml`).

Construida con Slint por su apariencia moderna out-of-the-box y bajo peso.

## Autoarranque e instalación

- El **daemon** se instala como servicio de usuario systemd:
  `~/.config/systemd/user/wallpaper-changer-daemon.service`, habilitado con
  `systemctl --user enable --now wallpaper-changer-daemon`. Esto da reinicio automático
  ante fallos y logs centralizados vía journal.
- La **GUI** no autoarranca: se lanza manualmente desde la bandeja del sistema (acción
  "Abrir configuración") o desde el lanzador de aplicaciones del sistema.
- Un script `install.sh` compila el workspace en modo release (`cargo build
  --release`), copia los binarios a `~/.local/bin/`, instala el archivo `.service`, y
  ejecuta `systemctl --user daemon-reload` + `enable --now`.

## Testing

- **`core`**: tests unitarios para el escáner de carpetas (filtrado por extensión, solo
  nivel superior), la lógica de selección aleatoria sin repetición (baraja/consume/re-
  baraja), y la (de)serialización de `config.toml`/`state.toml`.
- **Backend de KDE**: se testea el armado del script D-Bus/`evaluateScript` de forma
  aislada (sin depender de una sesión Plasma real corriendo), dejando la verificación
  de que efectivamente cambia el fondo como prueba manual en el entorno real del
  usuario (KDE Plasma en Fedora).
- **Daemon**: tests de integración sobre la máquina de estados del temporizador
  (reinicio de conteo ante cambio de config o `change_now_request`, pausa/reanudación),
  usando un backend de wallpaper fake que solo registra llamadas.
- **GUI**: verificación manual de la ventana (Slint no se presta bien a testing
  automatizado de UI para este alcance); se valida que lee/escribe correctamente
  `config.toml` y lee `state.toml`.
