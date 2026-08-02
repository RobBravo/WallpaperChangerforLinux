# Roadmap

Plan de trabajo priorizado para lo que falta del proyecto actual (KDE Plasma,
monitor único) y las mejoras grandes pedidas: soporte multipantalla, y
soporte para GNOME y XFCE.

**Cómo se va a ejecutar cada fase:** este documento es un mapa de prioridades
y alcance aproximado, no planes de implementación listos para ejecutar. Cuando
empecemos cada fase, pasa primero por una sesión de brainstorming propia
(según el flujo que ya usamos: idea → spec en `docs/superpowers/specs/` →
plan en `docs/superpowers/plans/` → implementación con TDD y revisión). Las
fases 2-4 en particular tienen decisiones de diseño reales que todavía no
están tomadas — se listan como preguntas abiertas en cada sección.

---

## Fase 0 — Cerrar el trabajo actual (P0, prioridad inmediata)

Nada de esto es una feature nueva: son huecos de verificación y deuda técnica
ya identificados en las revisiones de código anteriores, documentados pero no
resueltos. Bajo esfuerzo, bajo riesgo — tiene sentido cerrarlos antes de
empezar features grandes sobre una base sin terminar de verificar.

**Verificación pendiente del plan original (Task 14):**
- [x] Confirmar que el wallpaper **no** rota mientras el daemon está pausado
      (verificado en vivo: 2026-08-02).
- [ ] Confirmar autoarranque real: cerrar sesión/reiniciar y ver que el
      daemon arranca solo, sin intervención manual.
- [x] Confirmar que vaciar la carpeta configurada no rompe el daemon (debe
      loguear "no wallpapers found" y seguir corriendo — verificado en vivo:
      2026-08-02).

**Deuda técnica aparcada en revisiones anteriores:**
- [x] Escrituras no atómicas en `config.toml`/`state.toml` — arreglado con
      `wallpaper_core::fs_util::atomic_write` (escribe a un temporal y hace
      `rename`). Verificado en vivo: toggles rápidos de pausa ya no producen
      errores de parseo (2026-08-02).
- [x] Procesos zombie al reutilizar una instancia de la GUI ya abierta —
      arreglado con `reap_in_background` en `daemon/src/tray.rs`. Verificado
      en vivo: 8 relanzamientos rápidos, cero zombies (2026-08-02).
- [x] Race de instancia única en la GUI — arreglado con un `flock` exclusivo
      y no bloqueante (`std::fs::File::try_lock`, sin dependencias nuevas).
      Verificado en vivo: recuperación limpia tras un crash simulado
      (2026-08-02).
- [x] Archivo `.desktop` para lanzar la GUI desde el menú de aplicaciones —
      agregado (`packaging/wallpaper-changer-gui.desktop`, instalado por
      `install.sh` con la ruta real sustituida).

**Esfuerzo estimado:** un par de horas de trabajo real + verificación manual.

---

## Fase 1 — Soporte multipantalla (P1)

**Estado actual:** el backend de KDE ya no se cae con varios monitores — el
script que aplica el fondo (`core/src/kde_backend.rs::build_wallpaper_script`)
recorre `desktops()` y pone la **misma imagen en todos los monitores**. Lo
que falta es que cada monitor tenga su propia carpeta y su propia rotación
independiente, que es lo que la mayoría de la gente espera al pedir "soporte
multipantalla".

**Por qué va antes que GNOME/XFCE:** el formato de `config.toml`/`state.toml`
tiene que cambiar para modelar "N monitores, cada uno con su config" en vez
de un solo `folder`/`interval`. Ese cambio de esquema lo va a tener que
respetar **cualquier backend futuro**, incluidos GNOME y XFCE. Conviene
diseñarlo una sola vez, sobre el único backend que ya funciona, antes de
sumar más backends que tendrían que adaptarse al esquema viejo y después al
nuevo.

**Alcance aproximado:**
- Rediseño de `Config`/`State` en `wallpaper-core` para soportar una lista de
  monitores (identificados de forma estable — KDE expone nombres de salida
  vía `desktops()[i].screen` / `QScreen::name()`, hay que confirmar la forma
  más confiable de identificarlos entre sesiones).
- `wallpaper_core::scanner`/`queue` pasan de operar sobre una carpeta a
  operar sobre N carpetas (una cola de rotación independiente por monitor).
- `KdePlasmaBackend::set_wallpaper` deja de aplicar la misma imagen a todos
  los `desktops()` — aplica una imagen específica al desktop que corresponde
  a cada monitor.
- GUI: la ventana de configuración pasa de un formulario único a uno por
  monitor detectado (pestañas, o una lista con un formulario por monitor).
- Manejo de conectar/desconectar un monitor en caliente (¿qué pasa con su
  configuración guardada si se desconecta y se reconecta después?).

**Preguntas abiertas para la sesión de brainstorming de esta fase:**
- ¿Un monitor puede compartir carpeta con otro, o siempre son independientes?
- ¿Qué pasa la primera vez que se detecta un monitor nuevo — copia la config
  del monitor principal, o arranca con los valores por defecto?

**Esfuerzo estimado:** la fase más grande de las tres — toca los tres crates
(`core`, `daemon`, `gui`) y cambia el formato de los archivos de config
existentes (necesita migración o versión del esquema).

---

## Fase 2 — Soporte GNOME (P2)

GNOME configura el fondo de pantalla vía `gsettings`, no D-Bus a un shell
scriptable como Plasma — mecanismo completamente distinto.

**Alcance aproximado:**
- Nuevo backend `core/src/gnome_backend.rs` implementando el mismo trait
  `WallpaperBackend` ya definido en `core/src/backend.rs` — la separación
  daemon/GUI y el resto de `wallpaper-core` no deberían necesitar cambios.
- Mecanismo: `gsettings set org.gnome.desktop.background picture-uri
  'file:///...'` (y `picture-uri-dark` para temas oscuros en GNOME 42+) —
  probablemente invocando el binario `gsettings` vía `std::process::Command`,
  o hablando directo con `dconf`/D-Bus si se quiere evitar el binario externo
  (a decidir en el brainstorming de esta fase).
- GNOME expone monitores de forma distinta a KDE (`gnome-monitor-config`) —
  si la Fase 1 ya está implementada, este backend tiene que mapear su propio
  modelo de monitores al esquema genérico que definió esa fase.
- Detección del entorno de escritorio en tiempo de ejecución para elegir
  backend (`$XDG_CURRENT_DESKTOP`), o build separado por DE — **decisión de
  arquitectura pendiente**, afecta cómo se estructura `main()` en el daemon.

**Esfuerzo estimado:** medio — el mecanismo es más simple que el de KDE (sin
scripting), pero agrega la pregunta de detección de entorno que hoy no
existe (el proyecto asume KDE siempre).

---

## Fase 3 — Soporte XFCE (P2, junto con GNOME)

XFCE usa `xfconf-query`, con la complicación de que la ruta de la propiedad
incluye el monitor **y el workspace** (ej.
`/backdrop/screen0/monitor0/workspace0/last-image`), y varía según cuántos
monitores/workspaces tenga configurados el usuario.

**Alcance aproximado:**
- Nuevo backend `core/src/xfce_backend.rs`, mismo trait `WallpaperBackend`.
- Enumerar las propiedades reales de xfconf en tiempo de ejecución (no se
  pueden asumir rutas fijas) y escribir la imagen en cada una — vía
  `xfconf-query -c xfce4-desktop -p <ruta> -s <archivo>` o la librería
  `libxfconf` si existe un binding de Rust razonable.
- Mismo punto de la Fase 1: si hay soporte multipantalla, este backend
  necesita mapear los workspaces/monitores de XFCE al modelo genérico.

**Esfuerzo estimado:** el más incierto de los tres — la variabilidad de rutas
de xfconf según la configuración del usuario es la parte más delicada, vale
la pena investigarla a fondo en el brainstorming antes de comprometerse a un
diseño.

---

## Resumen de prioridad

| Fase | Qué | Prioridad | Tamaño |
|---|---|---|---|
| 0 | Verificación pendiente + deuda técnica | P0 | Chico |
| 1 | Multipantalla (KDE) | P1 | Grande |
| 2 | Soporte GNOME | P2 | Mediano |
| 3 | Soporte XFCE | P2 | Mediano-grande (incertidumbre) |
