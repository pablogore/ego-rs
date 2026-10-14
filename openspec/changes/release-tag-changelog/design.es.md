# Diseño: release-tag-changelog — Corte de release en `main` + integridad del backport de hotfix

> Compañero en español. Canónico / inglés: `design.md` (encabezados 1:1).
> Fuentes: `proposal.md`, `research.md`. `specs/` todavía no está escrito (sdd-spec corre en paralelo);
> este diseño deriva de `proposal.md` y debe reconciliarse con los delta specs antes de las tareas.

## Technical Approach

Cuatro archivos aditivos más una sección de documentación. Nada se genera, commitea ni empuja dentro
del árbol del repositorio.

- `release-automation` — `.github/workflows/release.yml` con `push: branches: [main]`. Un binario
  git-cliff fijado calcula la versión y renderiza las notas; se empuja un tag anotado a una
  referencia de **tag**; `gh release create --verify-tag` publica las notas como cuerpo del Release.
- `branch-promotion-integrity` — `.github/scripts/backport-drift.sh` (git puro, equivalencia de
  parche) ejecutado por `.github/workflows/backport-drift.yml` (cron + `workflow_dispatch`),
  reportando en un único issue de seguimiento que se actualiza; más la sección de ramas/hotfix en
  `CONTRIBUTING.md`.

## Design Invariant: no loop, no bypass

**Ningún paso de ninguno de los dos workflows empuja a una referencia de rama.** Es un invariante, no
una suposición, y se sostiene sobre tres hechos independientes — cualquiera de ellos por sí solo
rompe el bucle:

1. El único push en todo el diseño es `git push origin "$VERSION"` — un refspec de tag explícito.
2. El trigger `push:` de `production-gate.yml` lleva un filtro `branches:`, que nunca coincide con un
   push de tag.
3. Los pushes hechos con el `GITHUB_TOKEN` por defecto no vuelven a disparar workflows (research §5).

Los permisos son de mínimo privilegio por workflow: release `contents: write`; drift `contents: read`
+ `issues: write`. Sin PAT, sin GitHub App, sin cambios en la protección de ramas.
`production-gate.yml` no se edita.

## Architecture Decisions

| # | Elección | Rechazado | Justificación |
|---|---|---|---|
| AD-1 | git-cliff como binario de release fijado (curl + chmod, env `GIT_CLIFF_VERSION`) | `orhun/git-cliff-action` | Coincide con el precedente de `production-gate.yml` / `shipwright-validation.yml`, y el *mismo* comando corre localmente — que es lo que vuelve real la estrategia de verificación por dry-run en lugar de ser solo de CI |
| AD-2 | `cliff.toml` en la raíz del repositorio con `[bump] breaking_always_bump_major = false`, `features_always_bump_minor = true`, `initial_tag = "v0.1.0"`; `[git] tag_pattern = "v[0-9]*"` | Los valores por defecto de git-cliff; configuración bajo `.github/` | Los valores por defecto llevan un `!`/BREAKING directamente a `1.0.0`, violando la línea de alcance "permanecer en `0.x`". En la raíz, un `git cliff` local sin flags reproduce el CI byte a byte, sin un `--config` que olvidar |
| AD-3 | Notas renderizadas a `$RUNNER_TEMP/notes.md`, consumidas con `--notes-file` | La salida `content` de la action / heredoc multilínea en `$GITHUB_OUTPUT` | Sin escapado de delimitadores, y el texto arbitrario de los commits nunca cruza una superficie de interpolación. `$RUNNER_TEMP` está fuera del worktree, así que las notas no pueden commitearse ni por accidente |
| AD-4 | `git tag -a` + `git push origin <tag>`, luego `gh release create --verify-tag` | Dejar que `gh release create` cree el tag | La API de Releases crea un tag **ligero**; la propuesta exige uno anotado. `--verify-tag` falla de forma ruidosa en lugar de crear uno en silencio si el paso de push llegara a regresionar |
| AD-5 | CLI `gh` para la API de Release, `GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}` | `softprops/action-gh-release` | Viene preinstalado en los runners; la autenticación es idéntica en ambos casos, así que una Action de terceros solo agregaría una arista de cadena de suministro para reemplazar un comando |
| AD-6 | Baseline `v0.1.0` sembrado una vez, a mano, documentado en `CONTRIBUTING.md`; `initial_tag` como fallback determinista | Un job de bootstrap con `workflow_dispatch` | Sería código muerto tras una sola ejecución, y el riesgo principal de la propuesta pide explícitamente que el primer corte lo verifique una persona. El fallback hace que una primera corrida sin semilla igual dé `v0.1.0` en vez de un error |
| AD-7 | `git cherry -v origin/develop origin/main`; las líneas `+` son drift | `git log develop..main` | `git cherry` compara por patch-id, así que un backport por cherry-pick aparece como `-`, cumpliendo el contrato explícito de la propuesta "un cherry-pick no es drift". La comparación por identidad de commit da falso positivo en todo backport. Los merge commits quedan excluidos por construcción |
| AD-8 | Mantener un único issue abierto con la etiqueta `backport-drift` (crear / `gh issue edit --body-file` / cerrar cuando está limpio); el job siempre sale con `0` | Fallar el job; abrir un PR de backport automático | Una corrida de cron en rojo no lleva contenido y es fácil de ignorar; un issue es durable, asignable, legible por humanos y se auto-cierra. El PR automático está explícitamente fuera de alcance |
| AD-9 | Ventana de backport sin estado: filtro `BACKPORT_WINDOW_HOURS: "72"` sobre la fecha de commit | Un archivo de estado "ya visto" o historial vía API | `develop` está legítimamente atrasado durante la ventana; un filtro por antigüedad elimina toda esa clase de falsos positivos en cinco líneas y sin estado que se pueda corromper |

## Data Flow

```
merge PR → main
   ├─→ production-gate.yml            (sin cambios, push: branches: [main])
   └─→ release.yml                    (push: branches: [main], concurrency: cancel-in-progress false)
         checkout fetch-depth: 0, fetch-tags: true     ← git-cliff necesita historial + tags completos
         VERSION=$(git cliff --bumped-version)
         [ ¿el tag $VERSION ya existe? → exit 0 ]      ← idempotencia ante re-ejecución
         git cliff --unreleased --tag "$VERSION" -o "$RUNNER_TEMP/notes.md"
         git tag -a "$VERSION" ; git push origin "$VERSION"   ← SOLO REFERENCIA DE TAG
         gh release create "$VERSION" --verify-tag --notes-file "$RUNNER_TEMP/notes.md"
                │
                └── push de tag: no coincide con `branches:` → no re-dispara → no hay bucle

cron (diario) / workflow_dispatch
   └─→ backport-drift.yml
         git fetch origin main develop
         .github/scripts/backport-drift.sh   → markdown por stdout, VACÍO si está limpio, exit 0
              git cherry origin/develop origin/main │ '^+' │ antigüedad > ventana
         gh issue create | edit --body-file | close   (etiqueta: backport-drift)
```

## File Changes

| Archivo | Acción | Descripción |
|---|---|---|
| `.github/workflows/release.yml` | Crear | Corte de release en push a `main` |
| `.github/workflows/backport-drift.yml` | Crear | Reporte de drift programado y disparable a mano, no bloqueante |
| `.github/scripts/backport-drift.sh` | Crear | La única lógica real; testeable de forma aislada |
| `.github/scripts/release-guards.test.sh` | Crear | Casos del script de drift + aserciones estáticas de invariantes |
| `cliff.toml` | Crear | Semántica de bump y agrupación del changelog |
| `CONTRIBUTING.md` | Modificar | Nueva sección `## Branching, Releases, and Hotfixes` después de `## CI: Production Gate` |
| `.github/workflows/production-gate.yml` | Sin cambios | Triggers y jobs intactos |
| `Cargo.toml` raíz + los 22 miembros | Sin cambios | Ningún campo de versión se toca |

Estructura de la sección de `CONTRIBUTING.md` (siguiendo el estilo de bloques de comandos que el
archivo ya usa): `### Branching model` (feature → `develop` → `main`; hotfix desde `main` → `main` →
backport obligatorio) · `### Cutting a release` (automático al mergear en `main`; qué esperar; los
comandos de siembra única del tag baseline) · `### Backport drift check` (qué significa el issue
`backport-drift` y que la solución es `git cherry-pick` sobre `develop` — equivalente por parche, así
que la siguiente corrida cierra el issue sola).

## Interfaces / Contracts

```bash
# Versión + notas — idéntico local y en CI (AD-1, AD-2)
VERSION="$(git cliff --bumped-version)"
git cliff --unreleased --tag "$VERSION" -o "$RUNNER_TEMP/notes.md"

# Drift — '+' = no hay commit equivalente por parche en develop, '-' = ya está backporteado (AD-7)
git cherry -v origin/develop origin/main
```

Contrato de `backport-drift.sh`: recibe opcionalmente `<upstream-ref> <head-ref>` (por defecto
`origin/develop origin/main`), lee `BACKPORT_WINDOW_HOURS` (por defecto `72`), opera únicamente sobre
el repositorio del **cwd**, escribe un reporte markdown a stdout y siempre sale con `0`. **Stdout está
vacío cuando no hay drift** — toda la decisión de quien lo invoca es `[ -s report.md ]`.

## Testing Strategy

| Capa | Qué se prueba | Enfoque |
|---|---|---|
| Unitaria (RED primero) | Clasificación de `backport-drift.sh` | `.github/scripts/release-guards.test.sh` construye repositorios descartables bajo `mktemp -d` y verifica cuatro casos: mergeado normalmente → vacío, cherry-pickeado → vacío, sin backportear y antiguo → reportado, sin backportear y reciente → vacío (dentro de la ventana). Solo git + bash, sin red, sin `gh` |
| Unitaria (RED primero) | Los invariantes de bucle e inyección | El mismo script, con aserciones estáticas sobre ambos archivos de workflow: ninguna línea `git push` apunta a una referencia de rama, y ningún bloque `run:` contiene una interpolación `${{ github.event… }}` |
| Integración | Versión y notas de git-cliff | Dry-run local documentado de los dos comandos de arriba contra un clon real; verificar la versión calculada y revisar las notas a ojo. No se escribe test — esa lógica es de git-cliff, no nuestra |
| Manual (solo el primer corte) | Release de punta a punta | Checklist: sembrar el tag baseline → observar la primera corrida automática → exactamente un tag y un Release → `production-gate` **no** se volvió a ejecutar por el push del tag → `git log origin/main` sin cambios por el workflow → re-ejecutar sobre el mismo SHA y confirmar que no hay segundo tag ni segundo Release |

`act` se consideró y se descartó: no puede ejecutar fielmente llamadas a la API con `gh` ni pushes de
tags, así que una corrida verde de `act` no probaría nada que el test de bash no pruebe ya. El TDD
estricto aplica al único componente con lógica ramificada (`backport-drift.sh`) y a las aserciones de
invariantes; el cableado YAML y las invocaciones de herramientas externas no tienen rama que testear
en rojo.

## Threat Matrix

| Frontera | Casos adversarios mínimos | Aplicabilidad | Respuesta de diseño | Tests RED planificados |
|---|---|---|---|---|
| Rutas tipo documentación | `requirements.txt`, Markdown ejecutable, `README.sh` | N/A — ningún archivo se clasifica ni se ejecuta por su contenido; los únicos archivos escritos son un archivo temporal de notas y un reporte por stdout | — | — |
| Selección de repositorio git | `git -C`, rutas relativas, rutas absolutas | Aplicable | El checkout del cwd es la única autoridad de repositorio. `backport-drift.sh` acepta **solo refs**, nunca rutas, y nunca hace `cd` ni usa `git -C` | Ejecutar el script con el cwd apuntando a un repositorio descartable; verificar que reporta el drift de ese repositorio y no el del externo |
| Estado de commit | staged, `commit -a`, índice vacío | N/A — ningún paso crea un commit en ninguna parte; el worktree nunca se modifica | — | — |
| Estado de push | rama de seguimiento, primer push, refspec explícito | Aplicable | El único push es un refspec de tag explícito `git push origin "$VERSION"`; sin `git push` pelado, sin referencia de rama, y la guarda de idempotencia sale antes de empujar un tag ya existente | Aserción estática de que ningún `git push` de los workflows apunta a una referencia de rama; verificación manual de re-ejecución sobre el mismo SHA para la guarda |
| Comandos de PR | `--head` explícito, prefijo de entorno, comandos compuestos | Aplicable (comandos de issue) | Toda llamada a `gh` es un subcomando explícito con `--label`/`--body-file`; el cuerpo del issue viene de un archivo, nunca de interpolación en la línea de comandos, y `${{ }}` nunca aparece dentro de un cuerpo `run:` (los valores llegan por `env:`) | Aserción estática de que ningún bloque `run:` contiene `${{ github.event… }}` |

## Migration / Rollout

Orden: (1) sembrar el tag baseline a mano sobre el head actual de `main` y verificar a mano un
`git cliff` local en dry-run, (2) mergear el slice 1 — la siguiente promoción corta el primer release
automático bajo observación, (3) el slice 2 es independiente y puede aterrizar en cualquier orden. El
rollback es borrar archivos; el tag y el Release se pueden eliminar sin tocar el código fuente.

## Open Questions

- [ ] git-cliff `2.14.1` y el nombre de su asset para linux provienen de un tracker secundario
      (research S5). Apply DEBE confirmar que el tag de release y el nombre del asset resuelven, y
      subir el pin si no.
- [ ] Reglas de protección de tags sobre `v*` (quién puede borrar o sobrescribir un tag publicado) —
      anotado, fuera de alcance.
