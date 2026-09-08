# Tareas: release-tag-changelog — Corte de release en `main` + integridad de backport de hotfixes

> Compañero en español / Canónico: `tasks.md` (numeración 1:1).
> El TDD estricto está activo en este repositorio (registro de `sdd-init`). La única lógica
> condicional de este cambio es `.github/scripts/backport-drift.sh`; su suite de pruebas
> (`.github/scripts/release-guards.test.sh`) es RED-first según la Fase 2. El cableado YAML y las
> invocaciones al CLI de terceros no tienen rama que probar en rojo (design.md, "Testing Strategy").
> Las dos porciones del proposal se mantienen, con la porción 2 dividida en dos PRs por la
> separación RED/GREEN y el presupuesto de revisión de 400 líneas.

## Hallazgos de reconciliación (spec ↔ design) — leer antes de la Fase 0

Spec y design se redactaron de forma independiente, cada uno solo a partir de `proposal.md`, y el
propio informe de design.md pidió explícitamente esta verificación. Se encontró un desajuste
genuino; condiciona la Fase 0.

1. **Desajuste genuino — decisión requerida.** `design.md`, AD-9, introduce una ventana sin estado
   `BACKPORT_WINDOW_HOURS` (72 por defecto) basada en la antigüedad del committer: un hotfix en
   `main` aún no respaldado (backported) en `develop` **no** se reporta mientras esté dentro de esa
   ventana. La propia tabla "Testing Strategy" de design lo confirma explícitamente ("fresh
   unbackported → empty (in-window)"). Pero el escenario "An un-backported hotfix is reported" de
   `branch-promotion-integrity/spec.md` no lleva calificador de antigüedad: *"GIVEN a hotfix commit
   landed on main and its change is absent from develop WHEN drift detection next runs THEN that
   hotfix's change is reported as drift"* — leído literalmente, un hotfix fresco (de 1 hora) sin
   respaldar debe reportarse. Es una diferencia de comportamiento observable, no de redacción. Ver
   Fase 0.
2. **Informativo — no es un desajuste.** El encargo de tareas cita "7 requisitos / 12 escenarios"
   para `release-automation` y "5 requisitos / 8 escenarios" para `branch-promotion-integrity`. Los
   archivos de spec, tal como se leyeron, contienen 6 requisitos / 10 escenarios y 5 requisitos / 7
   escenarios, respectivamente. No se encontró ninguna brecha de contenido frente a design — tratar
   los conteos del encargo como aproximados, no como evidencia de requisitos faltantes.
3. **Informativo — se añade tarea de verificación, no es un desajuste.** El requisito de
   `release-automation` "agrupado por tipo de Conventional Commit" se satisface con la plantilla de
   changelog **por defecto** de git-cliff; `design.md`, AD-2, solo fija claves `[bump]`/`[git]`, no
   una sobrescritura de plantilla `[changelog]`. Esto es consistente con el design (no se reclama
   ninguna sobrescritura), pero implica que el requisito se satisface por un valor por defecto no
   declarado en lugar de una línea de configuración explícita — la tarea 1.5 añade una comprobación
   manual para que esto no se descubra por primera vez en el primer release real.

## Pronóstico de Carga de Revisión

| Campo | Valor |
|-------|-------|
| Líneas modificadas estimadas | ~590 en total — PR1 ~180, PR2 ~305, PR3 ~105 |
| Riesgo de presupuesto de 400 líneas | Bajo — cada PR se pronostica muy por debajo de 400, con margen |
| PRs encadenados recomendados | Sí — 3 PRs, no un único PR |
| División sugerida | PR1 (release-automation, independiente) · PR2 (backport-drift.sh + suite de pruebas RED-first, independiente) · PR3 (backport-drift.yml + documentación de branching/hotfix, depende del script de PR2; secuenciar después de que PR1 se fusione para evitar que ambos PRs inserten en el mismo encabezado nuevo de `CONTRIBUTING.md`) |
| Estrategia de entrega | ask-on-risk |
| Estrategia de cadena | PR1 y PR2 se ramifican de forma independiente desde `develop`; PR3 se encadena sobre PR2 (dependencia de código: el script debe existir) y debe abrirse/rebasarse después de que PR1 se fusione (dependencia de archivo compartido, no de código) |

Decisión requerida antes de aplicar: **Sí** — por dos razones independientes: (a) el Hallazgo de
Reconciliación #1 anterior (ventana AD-9 frente al escenario literal de spec) debe resolverse antes
de poder redactar el caso "fresh unbackported" de la Fase 2 en un sentido u otro; (b) confirmar la
división encadenada de 3 PRs antes de que `sdd-apply` empiece a abrir ramas.

### Unidades de Trabajo Sugeridas

| Unidad | Objetivo | PR probable | Comando de prueba enfocado | Depende de |
|------|------|-----------|----------------------|------------|
| 0 | Resolver el desajuste AD-9 frente al escenario de spec | — (decisión, sin código) | — | ninguna |
| 1 | Corte de release en push a `main` + tag base + checklist manual del primer corte | PR1 | dry-run local de `git cliff --bumped-version` / `git cliff --unreleased` (sin arnés de CI — lógica del proveedor, según design) | Unidad 0 no requerida (porción independiente) |
| 2 | `backport-drift.sh` + `release-guards.test.sh` RED-first | PR2 | `bash .github/scripts/release-guards.test.sh` | Unidad 0 (condiciona el caso en ventana) |
| 3 | `backport-drift.yml` + documentación de branching/hotfix/drift-check | PR3 | `bash .github/scripts/release-guards.test.sh` (re-ejecutado, ahora cubriendo ambos archivos de workflow) | Unidad 2 (el script debe existir); secuenciar después de que la Unidad 1 se fusione (encabezado compartido de `CONTRIBUTING.md`) |

## Fase 0: Decisión de Reconciliación — Bloqueante, Sin Código

- [x] 0.1 Resolver el Hallazgo de Reconciliación #1: ya sea (a) modificar
      `branch-promotion-integrity/spec.md` + `spec.es.md` en el escenario "An un-backported hotfix is
      reported" para declarar explícitamente la permisividad de 72 horas (añadir un escenario
      pareado para el caso dentro de ventana, alineado con el propio lenguaje de casos de prueba del
      design), o (b) redefinir/eliminar AD-9 en `design.md` + `design.es.md` de modo que un hotfix
      fresco sin respaldar siga reportándose y se elimine la ventana. No iniciar la tarea 2.7 de la
      Fase 2 hasta que un lado se haya modificado y ambos documentos concuerden. **Resuelto vía la
      opción (a)**: el requisito ahora dice "A Main-Only Change Absent From Develop Is Reported As
      Drift, After A Grace Period" con dos escenarios (reportado una vez superado el periodo de
      gracia, no reportado aún dentro del periodo) — coincide exactamente con el AD-9 de design.md.
      `design.md`/`design.es.md` sin cambios.

## Fase 1: Automatización de Release (Porción 1) — PR1

Cubre los seis requisitos de `release-automation/spec.md`. Independiente de las Fases 0/2/3.

- [x] 1.1 Crear `cliff.toml` en la raíz del repo (AD-2): `[bump] breaking_always_bump_major = false`,
      `features_always_bump_minor = true`, `initial_tag = "v0.1.0"`; `[git] tag_pattern = "v[0-9]*"`.
      Dejar `[changelog]` con la plantilla por defecto de git-cliff — no confeccionar a mano una
      plantilla de agrupación; verificar el valor por defecto en 1.5 (satisface "Release Body Is
      Human-Readable Record Grouped By Conventional-Commit Type").
- [x] 1.2 Crear `.github/workflows/release.yml`: `on: push: branches: [main]`;
      `concurrency: { group: release, cancel-in-progress: false }`; `permissions: contents: write`;
      checkout con `fetch-depth: 0, fetch-tags: true` (git-cliff necesita el historial completo); fijar
      `GIT_CLIFF_VERSION` como variable de entorno, instalar vía curl + chmod (AD-1, mismo patrón que
      `SHIPWRIGHT_VERSION` + instalación curl/chmod de
      `.github/workflows/shipwright-validation.yml` en las líneas 24/50-51). Confirmar que el tag de
      release fijado de git-cliff y el nombre de su asset linux realmente resuelven (design "Open
      Questions") — actualizar el pin si el `2.14.1` estimado del tracker está desactualizado.
- [x] 1.3 Mismo archivo: `VERSION="$(git cliff --bumped-version)"`; guardia de idempotencia — si ya
      existe un tag llamado `$VERSION`, salir con 0 antes de hacer nada más (satisface "cut only, and
      exactly once, as a direct result of a push landing on main").
- [x] 1.4 Mismo archivo: `git cliff --unreleased --tag "$VERSION" -o "$RUNNER_TEMP/notes.md"` (AD-3);
      `git tag -a "$VERSION" -m "$VERSION"`; `git push origin "$VERSION"` — únicamente un refspec de
      tag explícito, nunca un `git push` desnudo ni ninguna referencia de rama (AD-4; esta es la
      línea de la que depende el invariante de diseño); `gh release create "$VERSION" --verify-tag
      --notes-file "$RUNNER_TEMP/notes.md"` (AD-5) con `GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}`
      suministrado vía `env:`, nunca interpolado dentro de una cadena `run:`.
- [x] 1.5 Dry-run manual, sin push: ejecutar localmente `git cliff --bumped-version` y `git cliff
      --unreleased --tag <esa-versión>` contra el head actual de `main`; confirmar que las notas
      renderizadas están agrupadas por tipo de Conventional Commit (cierra el Hallazgo de
      Reconciliación #3) y registrar la versión calculada en la descripción del PR (design "Testing
      Strategy", fila Integration; riesgo del proposal "Version derivation misbehaves under 0.x
      semantics").
- [x] 1.6 `CONTRIBUTING.md`: añadir `## Branching, Releases, and Hotfixes` después de `## CI:
      Production Gate` (ancla confirmada, línea 18 actual) con solo la subsección `### Cutting a
      release` en este PR — qué dispara un release, que es completamente automático, y los comandos
      de siembra del tag base de una sola vez (`git tag -a v0.1.0 <main-sha> -m v0.1.0 && git push
      origin v0.1.0`, AD-6). Dejar `### Branching model` y `### Backport drift check` para PR3
      (tabla "File Changes" de design).
- [x] 1.7 Misma subsección: añadir la checklist de verificación manual solo-para-el-primer-release,
      transcrita de la fila Manual de "Testing Strategy" de design — sembrar el tag base; observar la
      primera ejecución automatizada; confirmar exactamente un tag nuevo y un Release nuevo;
      confirmar que `production-gate.yml` **no** se reejecutó a causa del push del tag; confirmar que
      `git log origin/main` permanece sin cambios por el workflow; reejecutar el workflow contra el
      mismo SHA de `main` y confirmar que no se crea un segundo tag/Release. Esta es la "checklist de
      verificación manual para el primer release real" requerida por la lista de tareas y la
      mitigación del riesgo principal del proposal.
- [ ] 1.8 Ejecutar la siembra del tag base a mano sobre el head actual de `main` (design "Migration /
      Rollout" paso 1), antes o inmediatamente después de fusionar PR1. Registrar el tag sembrado y
      su SHA en la descripción del PR.
- [x] 1.9 Verificación: la revisión confirma que ningún paso `run:` en `release.yml` interpola
      `${{ github.event... }}` (design Threat Matrix, fila "PR commands" — los valores deben llegar
      vía `env:`), y que el único `git push` del archivo apunta a `"$VERSION"`, nunca a una
      referencia de rama (hecho 1 del invariante de diseño).

## Fase 2: Script de Backport Drift + Suite de Pruebas RED-First — PR2

Cubre los requisitos de reporte de drift y equivalencia de parche de
`branch-promotion-integrity/spec.md` a nivel de script. Independiente de la Fase 1 (sin solapamiento
de archivos). Condicionada por la Fase 0 solo para la tarea 2.7.

- [ ] 2.1 RED: crear `.github/scripts/release-guards.test.sh`. Primer caso: construir un repositorio
      desechable bajo `mktemp -d`, aterrizar un hotfix en `main`, fusionarlo en `develop`
      normalmente, ejecutar `.github/scripts/backport-drift.sh` (aún no existe) contra él, afirmar
      stdout vacío y código de salida `0`. Confirmar que la prueba falla solo porque falta el script.
- [ ] 2.2 GREEN: crear `.github/scripts/backport-drift.sh`. Firma: `<upstream-ref> <head-ref>`
      opcionales (por defecto `origin/develop origin/main`); lee `BACKPORT_WINDOW_HOURS` (por defecto
      `72`); nunca usa `git -C` ni hace `cd` — opera solo sobre el repositorio del cwd (design Threat
      Matrix, fila "Git repository selection"); ejecuta `git cherry -v "$upstream" "$head"`, trata
      las líneas prefijadas con `+` como drift (AD-7); escribe un reporte markdown en stdout (vacío
      cuando está limpio); siempre sale con `0`. Hacer pasar 2.1.
- [ ] 2.3 RED: añadir el caso de cherry-pick — hotfix en `main`, respaldado en `develop` vía
      `git cherry-pick` (SHA de commit distinto, mismo parche), afirmar stdout vacío.
- [ ] 2.4 GREEN: confirmar que 2.3 pasa contra la implementación 2.2 sin modificar (la comparación
      por patch-id ya lo cubre según AD-7); si falla, corregir la lógica de clasificación.
- [ ] 2.5 RED: añadir el caso "old unbackported" — hotfix en `main` con fecha de committer más
      antigua que `BACKPORT_WINDOW_HOURS` y sin commit equivalente en `develop`; afirmar stdout no
      vacío que nombre el commit.
- [ ] 2.6 GREEN: implementar el filtro de antigüedad de committer usando `BACKPORT_WINDOW_HOURS` en
      `backport-drift.sh`; hacer pasar 2.5.
- [ ] 2.7 RED: añadir el caso "fresh unbackported, in-window", según la decisión resuelta en la Fase
      0 — afirmar stdout vacío si la ventana de AD-9 se mantiene, o afirmar stdout no vacío si la
      Fase 0 la eliminó/redujo. No redactar este caso antes de la decisión de la Fase 0.
- [ ] 2.8 GREEN: confirmar que 2.7 pasa contra el comportamiento resuelto en la Fase 0; ajustar el
      filtro de antigüedad si la Fase 0 lo cambió.
- [ ] 2.9 RED: caso de aislamiento de selección de repositorio (design Threat Matrix) — invocar el
      script con el cwd apuntando a un repositorio de scratch distinto de cualquier checkout externo;
      afirmar que reporta solo el drift de ese repositorio de scratch.
- [ ] 2.10 GREEN: confirmar que 2.9 pasa (debería cumplirse ya dado que 2.2 nunca usa `git -C` ni
      rutas absolutas; añadir una corrección de regresión solo si falla).
- [ ] 2.11 RED+GREEN: aserciones de invariantes estáticos (design "Testing Strategy", fila
      loop/injection) en el mismo archivo de pruebas — hacer grep sobre `release.yml` (ya fusionado
      desde PR1, o presente en esta rama) para afirmar que ninguna línea `git push` apunta a una
      referencia de rama, y que ningún bloque `run:` contiene `${{ github.event... }}`. Acotar la
      aserción de este PR solo a `release.yml` — `backport-drift.yml` no existe hasta PR3;
      reejecutar la aserción completa en la Fase 3.
- [ ] 2.12 Verificación: `bash .github/scripts/release-guards.test.sh` verde de principio a fin;
      confirmar que no hay llamada de red ni invocación de `gh` en ningún lugar de la suite
      (restricción de design — git/bash puro).

## Fase 3: Workflow de Backport Drift + Documentación de Branching/Hotfix — PR3

Cubre los requisitos restantes de `branch-promotion-integrity/spec.md` (política documentada,
programación recurrente, no bloqueante de extremo a extremo). Depende de PR2 (el script debe
existir). Secuenciar después de que PR1 se fusione — no es una dependencia de código, pero ambos PRs
insertan subsecciones en el mismo encabezado nuevo de `CONTRIBUTING.md`.

- [ ] 3.1 Crear `.github/workflows/backport-drift.yml`: `on: { schedule: [{ cron: ... }],
      workflow_dispatch: {} }`; `permissions: { contents: read, issues: write }`;
      `git fetch origin main develop`; ejecutar `.github/scripts/backport-drift.sh > report.md`.
- [ ] 3.2 Mismo archivo: `[ -s report.md ]` decide crear/actualizar vs. cerrar — upsert de un único
      issue abierto etiquetado `backport-drift` (`gh issue create` / `gh issue edit --body-file` /
      `gh issue close`, AD-8); el job siempre sale con `0` independientemente del contenido del
      reporte (satisface "non-blocking, report-only, no automated remediation").
- [ ] 3.3 `CONTRIBUTING.md`: añadir `### Branching model` bajo el encabezado existente
      `## Branching, Releases, and Hotfixes` — las ramas de feature promueven a `develop` vía PR;
      `develop` promueve a `main` vía PR; un hotfix se ramifica desde `main`, aterriza en `main` vía
      PR, y DEBE respaldarse posteriormente en `develop` (satisface "branching model and hotfix
      obligation are documented").
- [ ] 3.4 Mismo archivo: añadir `### Backport drift check` — qué significa el issue
      `backport-drift`, que la corrección es un `git cherry-pick` sobre `develop` (equivalente en
      parche, así que la siguiente ejecución programada cierra el issue por sí sola), y que la
      verificación nunca bloquea un merge ni un push.
- [ ] 3.5 Reejecutar las aserciones de invariantes estáticos de 2.11, ahora acotadas tanto a
      `release.yml` como a `backport-drift.yml`; extender `release-guards.test.sh` en el mismo lugar
      en vez de duplicar la comprobación en un segundo archivo.
- [ ] 3.6 Verificación: confirmar que el encabezado fusionado de `CONTRIBUTING.md` contiene las tres
      subsecciones en el orden que especifica design — `### Branching model` → `### Cutting a
      release` → `### Backport drift check`. Si PR3 aterriza con PR1 ya fusionado, esto es una simple
      comprobación de orden; si el orden de fusión se invirtió, mover la subsección de PR1 a la
      posición correcta en este PR.

## Criterios de Aceptación Transversales (aplican a todos los PRs anteriores, no se repiten)

- Ningún commit se crea o empuja jamás a `main` o `develop` por ningún workflow de este cambio.
- Ninguna línea `git push` en ningún archivo de workflow apunta a una referencia de rama —
  únicamente el refspec de tag explícito en `release.yml`.
- Ningún bloque `run:` en ninguno de los dos workflows interpola `${{ github.event... }}`
  directamente; los valores llegan vía `env:`.
- Ningún manifiesto de crate (`Cargo.toml`, raíz o cualquiera de los 22 miembros) se toca en ningún
  punto de este cambio.
- `.github/workflows/production-gate.yml` no se edita en ningún PR de este cambio.

## Fuera de Alcance (reafirmado, no se relitiga)

Versionado por crate o publicación en crates.io; un `CHANGELOG.md` rastreado en el árbol del
repositorio; pull requests de backport auto-abiertos o cualquier comportamiento de bloqueo de merge
desde la detección de drift; cambios de protección de rama, una app/PAT de bypass, o requisitos de
commits firmados/historial lineal; ediciones al comportamiento de gating de `production-gate.yml` o
a cualquier lógica de promoción de `.shipwright/workflow.yaml`.
