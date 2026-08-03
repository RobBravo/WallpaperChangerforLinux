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

## Fase 1 — Soporte multipantalla (P1) — ✅ Completada (2026-08-02)

`Config`/`State` (en `wallpaper-core`) pasaron de un `folder`/`interval` plano
a una lista de monitores (`Vec<MonitorConfig>`/`Vec<MonitorState>`), cada uno
identificado por el UUID estable que KDE ya asigna vía KScreen (sobrevive
reinicios y cambios de puerto). `KdePlasmaBackend::set_wallpaper` ya no
escribe la misma imagen en todos los `desktops()` — calcula la posición
relativa del monitor destino (`core/src/kde_backend.rs::position_rank`) y
aplica solo ahí. El daemon (`daemon/src/engine.rs`, `daemon/src/main.rs`)
mantiene una cola de rotación y un deadline independiente por monitor, con
sondeo cada 30s para detectar conexión/desconexión en caliente. La GUI
(`gui/ui/app-window.slint`, `gui/src/main.rs`) reemplazó el formulario único
por un `ComboBox` selector de monitor (no `TabWidget` — Slint no soporta una
cantidad de tabs definida en tiempo de ejecución) y ahora vuelve a
re-detectar monitores conectados cada 5s mientras la ventana está abierta.

**Preguntas abiertas resueltas:** un monitor nunca comparte carpeta con otro
(siempre independientes); un monitor nuevo copia la config del monitor
principal si existe, o los valores por defecto originales si no.

**Verificado en vivo en esta máquina** (un solo monitor físico disponible):
instalación limpia con migración automática del `config.toml` plano viejo,
selector de monitor en la GUI, rotación automática (con dos bugs reales de
manejo de deadlines encontrados y corregidos durante esta verificación —
ver el historial de commits de `daemon/src/main.rs`), y pausa por monitor.

**No verificado — requiere un segundo monitor físico:**
- Que "Cambiar ahora" solo afecte al monitor seleccionado (el mecanismo está
  probado por unit test, pero no visualmente en dos pantallas reales).
- Conectar/desconectar un monitor en caliente en hardware real.
- Que la correlación por posición (`position_rank` en Rust vs. el orden que
  genera `desktops().sort(...)` en el script JS) realmente coincida en una
  sesión KWin real con más de un monitor — lo único que el diseño marcó
  desde el principio como imposible de verificar sin ese hardware.

---

## Fase 2 — Soporte GNOME (P2) — ✅ Completada (2026-08-02)

GNOME configura el fondo de pantalla vía `gsettings`, no D-Bus a un shell
scriptable como Plasma. Investigación previa al diseño encontró un hecho
clave que no estaba contemplado al escribir esta sección originalmente:
**GNOME no soporta nativamente un wallpaper distinto por monitor** — la
clave `org.gnome.desktop.background`/`picture-uri` aplica una sola imagen a
todo el escritorio virtual, abarcando todos los monitores. Por eso, bajo
GNOME toda la app se comporta como una configuración compartida única —
igual que este proyecto se comportaba en KDE antes de la Fase 1 — en vez de
implementar composición de imágenes (como hacen herramientas de terceros
tipo HydraPaper).

`daemon/src/main.rs` y `gui/src/main.rs` detectan el escritorio actual una
sola vez al arrancar (vía `$XDG_CURRENT_DESKTOP`, manejando valores
compuestos como `"ubuntu:GNOME"`) y eligen el backend/función de detección
de monitores correspondiente en tiempo de ejecución — un solo binario, sin
builds separados por escritorio. Un escritorio no reconocido hace que el
daemon salga con un mensaje claro en vez de asumir KDE silenciosamente.
`core/src/gnome_backend.rs` aplica el wallpaper vía el binario `gsettings`
(sin shell, sin superficie de inyección — a diferencia del script D-Bus de
KDE, que sí necesita escapar la ruta).

**Verificado:** solo con tests automatizados (detección de escritorio,
selección de backend, construcción de los argumentos de `gsettings`,
etc.) — **ningún test se corrió en una sesión GNOME real**, porque no había
ninguna disponible durante el desarrollo. Si alguien con acceso a GNOME
quiere confirmar el comportamiento en vivo, esto es lo que falta probar:
que el ícono de bandeja y la GUI detectan GNOME correctamente, que
`gsettings` efectivamente cambia el fondo, y que la migración automática
de un `config.toml` viejo funciona igual que en KDE.

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
| 1 | Multipantalla (KDE) | P1 | Grande — ✅ Completada |
| 2 | Soporte GNOME | P2 | Mediano — ✅ Completada (sin verificar en vivo) |
| 3 | Soporte XFCE | P2 | Mediano-grande (incertidumbre) |
