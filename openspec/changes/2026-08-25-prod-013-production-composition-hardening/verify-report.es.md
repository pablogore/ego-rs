# Reporte de Verificación: PROD-013 — Endurecimiento de la Composición de Producción

> Compañero de revisión en español. Fuente canónica: `verify-report.md` (identificadores 1:1).

**Estado**: PASS
**Verificado contra**: `opsx/prod-013-wu7-architecture-docs` (acumula las 7 unidades de trabajo apiladas)
**Baseline en design.md**: `develop @ a740d34`

## Resumen Ejecutivo

Las 39 tareas de las 7 unidades de trabajo están funcionalmente completas y verificadas contra el código real (no contra reportes previos de apply). 4 de los 5 comandos de gate (`cargo check --workspace`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace` y el `run-suite` respaldado por Docker) terminan con exit 0; `cargo fmt --check` sigue en no-cero únicamente por una desviación de baseline preexistente en `develop`, verificada y sin relación con este cambio. Neto: **0 CRITICAL, 0 WARNING, 4 SUGGESTIONS no bloqueantes** (ver Hallazgos abajo). La verificación crítica #10 — si una composición `Profile::Production` puede seguir corriendo silenciosamente sobre almacenamiento volátil — no encontró ningún bypass: el gate está cerrado estructuralmente para reference-app (campo `profile` privado, dos constructores) y el único límite arquitectónico que sí existe (`AppBuilder::profile()` no se propaga a runtimes de entidad ya construidos) está documentado explícitamente en el código exactamente como lo exige AD-5 de `design.md`, y es el mismo riesgo residual que la propuesta (R-1) ya nombró y aceptó, no algo oculto.

## Completitud de Tareas (tasks.md)

| Fase | Tareas | Estado |
|---|---|---|
| 1 — `Profile` + predicado compartido | 1.1–1.5 | [x] todas, verificadas contra `profile.rs`, `error.rs`, re-export en `lib.rs` |
| 2 — Gate de `EntityRuntimeBuilder` + `try_build()` | 2.1–2.7 | [x] todas, verificadas contra `builder.rs` (tests + panic guard) |
| 3 — Gate del effect store | 3.1–3.7 | [x] todas, verificadas contra `service-sdk/runtime/builder.rs`, `app/mod.rs` |
| 4 — Cableado de `EntityEventStores` | 4.1–4.10 | [x] todas, verificadas contra `reference-app/src/lib.rs` |
| 5 — Fix de flavor de runtime Postgres | 5.1–5.3 | [x] todas, verificadas: 7 funciones de test en 4 archivos migradas a `flavor = "multi_thread"`; migración 012 confirmada presente en el baseline `develop` (heredada de WU1, no forma parte del diff de esta unidad) |
| 6 — Guardas de regresión AD-10 | 6.1–6.2 | [x] todas, verificadas: `production_profile_guard.rs` existe; assertion de Production presente en `durable_entity_progress_postgres.rs:102` |
| 7 — Documentación | 7.1–7.2 | [x] todas, verificadas en `ARCHITECTURE.md` y `ROADMAP.md` |
| 8 — Verificación final | 8.1, 8.2 sin marcar en tasks.md; 8.3 [x] | **Ejecutadas en este pase de verify** — ver Evidencia de Gates abajo. No es bloqueante: ambas tareas son literalmente "correr los comandos de gate", que es el trabajo propio de esta fase. Se recomienda que el orquestador/apply las marque `[x]` post-verify, pero es administrativo, no un hueco funcional. |

Ninguna tarea sin marcar produce CRITICAL: 8.1/8.2 son tareas de ejecución de comandos cuyo contenido este mismo pase de verify realizó y confirmó en verde.

## Evidencia de Gates

| Comando | Exit | Resultado |
|---|---|---|
| `cargo fmt --check` | no-cero | 1 diff, en `crates/service-sdk/src/app/mod.rs:275` (envoltura de línea de `record_app_started`). **Confirmado preexistente en el baseline `develop`** (`git show develop:crates/service-sdk/src/app/mod.rs` tiene la misma línea sin envolver) — no introducido por ningún commit de PROD-013 (`git diff develop..HEAD` para ese archivo no toca esa línea). No es una regresión de PROD-013; registrado como SUGGESTION, fuera del alcance de este cambio. |
| `cargo check --workspace` | 0 | Limpio. |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 | Limpio, cero warnings. |
| `cargo test --workspace` | 0 | 0 fallos en cada binario unit/integración/doc-test del workspace (totales de muestra incluyen 365, 248, 198, 180, 117, 85, 52, 42, 29, 28, 26×2, 22, 16, 15×2, 13×2, 12×2, 9×3, 8×2, 7×3, 6×5, 5×7, 4×8, 3×varios, 2×varios, 1×varios — todos `0 failed`). Confirma SC-7. |
| `run-suite` Docker (`DOCKER_HOST=unix:///Users/pablogore/.colima/default/docker.sock`) | 0 | 43 passed, 0 failed, 1 ignored (la exclusión preexistente/documentada del Tier-1 `postgres_satisfies_state_store_conformance`) — coincide exactamente con el resultado estabilizado que design.md reporta en la tarea 5.3. Incluye `entity_event_stores_wiring_postgres::{a_written_snapshot_survives_a_fresh_open_against_the_same_pool, opened_stores_declare_profile_production}` y los 3 tests de `durable_entity_progress_postgres` (con la assertion de Production de AD-10 en línea) — todos `ok`. |

## Matriz de Cumplimiento de Especificación

| Spec | Requisito | Evidencia de código | Evidencia de test | Estado |
|---|---|---|---|---|
| production-composition-hardening | Declaración explícita de Profile | enum `Profile` en `profile.rs`, `Default = Dev` | `require_configured_matrix` | PASS |
| production-composition-hardening | Gate del event store | `builder.rs:284-297` `validate_persistence` | `try_build_rejects_missing_event_store_under_production` | PASS |
| production-composition-hardening | Gate del snapshot store | idem | `try_build_rejects_missing_snapshot_store_under_production` | PASS |
| production-composition-hardening | Gate del effect store, condicionado a executor | `service-sdk/runtime/builder.rs:777-794` `validate_persistence_profile` | `validate_persistence_profile_rejects_missing_effect_store_when_executor_registered`, `validate_persistence_profile_ok_when_no_executor_registered` | PASS |
| production-composition-hardening | Config. parcial cubierta por los gates por capacidad | pliegue de AD-7, no existe check separado | `try_build_rejects_partial_configuration_under_production` | PASS |
| production-composition-hardening | Un validador, fuente única de verdad | `require_configured` (profile.rs:28), llamado desde ambos crates, sin duplicado por grep | `build_and_try_build_agree_on_persistence_profile_validation` | PASS |
| production-composition-hardening | Rechazos accionables | `PersistenceCompositionError::NotConfigured{capability,fix}` | afirmado en los mismos tests | PASS |
| production-composition-hardening | Composiciones no-Production no afectadas | default `Profile::Dev`, fallbacks `unwrap_or_else` intactos fuera de Production | `dev_profile_builds_on_nothing_configured`; `cargo test --workspace` 0 fallos nuevos | PASS |
| production-composition-hardening | Regla de completitud de persistencia documentada | nueva subsección en `ARCHITECTURE.md:192` | lectura directa | PASS |
| production-composition-hardening | Límite con PROD-005 documentado | `ARCHITECTURE.md:452-456`, `ROADMAP.md:675-677` | lectura directa | PASS |
| production-composition-hardening | Reference app declara el profile vía `EntityEventStores` | `lib.rs:345-440` campo privado, dos constructores | `EntityEventStores::in_memory().profile() == Dev` (`production_profile_guard.rs`), `opened_stores_declare_profile_production` (Docker) | PASS |
| production-composition-hardening | Snapshot store de producción de reference app es durable | `lib.rs:407-418` dos instancias `PostgreSQLSnapshotStore` | `a_written_snapshot_survives_a_fresh_open_against_the_same_pool` (Docker) | PASS |
| production-composition-hardening | Check de regresión guarda la declaración de referencia | `production_profile_guard.rs` (mitad Dev) + `durable_entity_progress_postgres.rs:102` (mitad Production) | ambos corrieron en verde | PASS |
| persistent-entity | Gatea el fallback en memoria por Profile | `builder.rs:323-326` `build()` panickea antes de delegar | `build_panics_on_same_condition_try_build_refuses` | PASS |
| persistent-entity | Config. parcial cubierta por los gates por capacidad | idem arriba | idem | PASS |
| persistent-entity | Los 67 call sites existentes no afectados | default `Profile::Dev`, cero ediciones en call sites | `cargo build --workspace` + `cargo test --workspace` en verde | PASS |
| application-composition | Declaración de Profile en la raíz de composición | `RuntimeBuilder::profile()` (builder.rs:282), `AppBuilder::profile()` (app/mod.rs:494) | tests unitarios `profile_...` | PASS |
| application-composition | Gate del effect store vía `CompositionError::Validation` | `RuntimeError::PersistenceNotConfigured(#[from] ...)` (runtime_builder.rs:1510+) → `CompositionError::Validation` (ruta preexistente, sin modificar) | test de integración en `crates/service-sdk/tests/` (tarea 3.7) | PASS |
| application-composition | Reference app propaga su profile, guardado | `lib.rs:735-739` captura `stores.profile()` antes del move; `App::builder()...profile(profile)` en la cadena | assertion de Production en Docker + guarda de Dev | PASS |

## Verificación Profunda de las 10 Comprobaciones Explícitas

1. **`profile.rs`** — `Profile{Dev(default), Production}` + `require_configured` existen exactamente como se diseñó (AD-1, AD-3). Confirmado por lectura directa.
2. **Gate en `builder.rs`** — `.profile()`, `try_build()`, `build()` panickea bajo `Profile::Production` con capacidad faltante. Confirmado que `cargo check --workspace` (equivalente para este propósito) está en verde con los 67 call sites de `EntityRuntimeBuilder::new()` intactos (verificado vía `git diff develop..HEAD`).
3. **`service-sdk/runtime/builder.rs`** — `validate_persistence_profile`, condicionado a `effect_executors.is_empty()`, llamado desde `build()` (builder.rs:820) y `try_build()` (builder.rs:1144). Confirmado.
4. **`examples/reference-app/src/lib.rs`** — `EntityEventStores::open()` cablea dos instancias reales de `PostgreSQLSnapshotStore` + `Profile::Production`; `in_memory()` cablea `InMemorySnapshotStore` + `Profile::Dev`. Confirmado byte a byte contra AD-8/AD-9.
5. **`crates/persistence/src/postgres/snapshot.rs` + migración 012** — ambos existen en el baseline `develop`, heredados de WU1/PR1 y no forman parte del diff de WU5 de PROD-013; `IS NOT DISTINCT FROM` usado en ambos `SELECT`, `INSERT ... ON CONFLICT` ramificado tenant/systemwide apuntando cada uno a su propio índice parcial (`ux_snapshots_identity_tenant`, `ux_snapshots_identity_systemwide`). Confirmado como precondición verificada sobre la que se apoya WU5, no algo que WU5 introduzca.
6. **Tests Postgres migrados a `flavor = "multi_thread"`** — confirmado por grep: exactamente las 7 funciones de test en los 4 archivos que nombran las tareas 5.1/5.2 (`durable_entity_progress_postgres.rs` ×3, `dual_aggregate_crash_recovery_postgres.rs` ×2, `concurrent_replicas_postgres.rs` ×1, `entity_event_stores_wiring_postgres.rs` ×1).
7. **`production_profile_guard.rs` + assertion de Production** — ambos existen y ambos pasan (verificado en las corridas de gate arriba).
8. **`ARCHITECTURE.md` / `ROADMAP.md`** — ambos llevan las nuevas secciones de PROD-013 (IS-9/IS-10). Confirmado.
9. **Ningún gate de read-side, real ni pseudo** — se grepeó `AppBuilder::projection`, `SharedReadSideStore`, `ReadSideSink` contra cada archivo que toca `profile`: cero intersección. `AppBuilder::projection()` sigue siendo DI intacto, `SharedReadSideStore`/`ReadSideSink` siguen siendo cableado local de reference-app sin ninguna conciencia de `Profile`. D-4/OOS-2 respetados por completo.
10. **El bypass motivador** — para reference-app (el host concreto que este cambio apunta), el bypass está cerrado **estructuralmente**, no por convención: `EntityEventStores.profile` es un campo privado, y sus únicos dos productores (`open()`, `in_memory()`) siempre emparejan el profile con los stores reales/volátiles correctos — no existe ningún valor construible de `EntityEventStores` que los desalinee. Para el framework en general, queda un límite arquitectónico intencional: `AppBuilder::profile(Production)` no valida retroactivamente un `EntityRuntime` ya construido (con defaults `Profile::Dev`) antes de registrarse vía `.entity()`. Esto no es un hueco silencioso — el doc comment de `AppBuilder::profile()` lo declara explícitamente (verificado palabra por palabra contra lo exigido por AD-5), y el propio R-1 de la propuesta nombra este riesgo residual exacto y lo acepta en vez de afirmar que lo cierra. **No es un hallazgo bloqueante** — es el límite documentado de una convención opt-in, no un hueco no reconocido.

## Coherencia de Diseño

Las 11 decisiones de arquitectura (AD-1 a AD-11) verificadas contra el código real, no solo la intención de diseño:
- AD-1 (ubicación de Profile + re-export): confirmado.
- AD-2 (forma de `PersistenceCompositionError`): confirmado, una sola variante `NotConfigured{capability,fix}`, sin dispersión de variantes por capacidad.
- AD-3 (`require_configured` como el único predicado): confirmado, llamado desde ambos crates, sin segundo check paralelo en ninguna parte de la ruta de composición (SC-8 satisfecho).
- AD-4 (`try_build()` refleja la forma de PROD-012): confirmado forma por forma, `self` no `mut self` (correctamente más angosto, según la diferencia declarada en el diseño).
- AD-5 (gate del effect store, condicional, no-propagación de `AppBuilder::profile` documentada): confirmado.
- AD-6 (sin puente entre capas para el rechazo de event/snapshot): confirmado — `observed_entity_runtime` devuelve `Result<_, PersistenceCompositionError>` absorbido por `?` en el `Box<dyn Error>` de `build_runtime_with`, sin `From` agregado.
- AD-7 (config. parcial plegada en el gate de Production, sin check separado): confirmado, el texto de spec coincide (`production-composition-hardening/spec.md`).
- AD-8 (el profile viaja en `EntityEventStores`): confirmado, campo privado, solo dos constructores.
- AD-9 (cableado de snapshot durable, dos instancias tipadas no una compartida): confirmado, `org_snapshot`/`user_snapshot` cada uno su propio `PostgreSQLSnapshotStore::new(pool.clone())`.
- AD-10 (dos assertions de test, no un lint de `xtask`): confirmado, `production_profile_guard.rs` + una línea en `durable_entity_progress_postgres.rs`.
- AD-11 (Approach C evaluado y diferido, no implementado): confirmado — no existe código de default-flip en ningún lado, `Profile::Dev` permanece `#[default]`.

## Hallazgos

**CRITICAL**: Ninguno.

**WARNING**: Ninguno.

**SUGGESTION** (no bloqueante, informativo — señalado por pedido explícito del orquestador de reportar deuda conocida con honestidad):

1. `cargo fmt --check` reporta un diff en `crates/service-sdk/src/app/mod.rs:275` (llamada a `record_app_started`). Confirmado preexistente en `develop` antes de que se ramificara PROD-013 (presente en `git show develop:...`, no tocado por ningún diff de commit de PROD-013). Desviación de entorno/versión de rustfmt no relacionada con este cambio; no es responsabilidad de este cambio arreglarla, pero se señala para que no se le atribuya erróneamente a PROD-013 más adelante.
2. La tabla `EXPECTED_PAIRS` de `integration-tests/tests/infrastructure/schema_index_assertion.rs` (que cubre `events`, `operation_reservations`, `operation_receipts`) **no** incluye la tabla `snapshots`, aunque la migración 012 (ya presente en el baseline `develop`, heredada de WU1 — no forma parte de este cambio) le aplica el mismo patrón de índice único parcial doble. Es un hueco de cobertura real en ese test de assertion, ya conocido y documentado como deuda aceptada según el brief del orquestador — no introducido como sorpresa por esta verificación, y explícitamente no bloqueante para este cambio.
3. `crates/persistence/src/postgres/repository.rs` (tabla `aggregates`) todavía usa el patrón no-null-safe `tenant_id = $2` en vez de `IS NOT DISTINCT FROM` — la misma clase de defecto que la migración 012 corrigió para `snapshots` en el baseline `develop`. Preexistente, fuera del alcance de PROD-013 (su conjunto de capacidades está fijado en exactamente tres: event/snapshot/effect store — D-3), y ya documentado como deuda de seguimiento aceptada según el brief del orquestador.
4. AD-11 (Approach C, la alternativa de invertir el default) permanece evaluado-y-diferido según D-7/OOS-5, tal como registra design.md; no existe tarea de implementación para ello y no se esperaba ninguna. No es un hueco — es un diferimiento deliberado y documentado.

## Veredicto

**PASS.** Las 39 tareas están funcionalmente completas y coinciden exactamente con el código tal como lo describen specs y design (incluyendo todas las correcciones de evidencia EC-1/EC-2 y las decisiones confirmadas AD-7/AD-8/AD-9). 4 de los 5 gates solicitados pasan con exit 0; `cargo fmt --check` sigue en no-cero únicamente por una desviación de baseline preexistente en `develop`, sin relación con este cambio. No existe ningún gate de read-side, real ni pseudo (restricción dura D-4/OOS-2 honrada). El bypass motivador está cerrado estructuralmente para reference-app, y el único límite arquitectónico restante en el framework general es intencional, está documentado exactamente como lo exigió design.md, y coincide con el riesgo residual que la propia propuesta ya nombró y aceptó (R-1) en vez de dejarlo abierto en silencio.

**Próxima fase recomendada**: `sdd-archive`.

## Aprendizajes Clave

1. El campo privado `profile` de `EntityEventStores` con exactamente dos constructores cierra R-1 estructuralmente para reference-app — más fuerte que la garantía de "una llamada más un check de regresión" que la propuesta pedía originalmente.
2. El resultado exacto de la suite respaldada por Docker (43 passed / 0 failed / 1 ignored) coincide precisamente con el número de estabilización que design.md reporta en la tarea 5.3, confirmando que la migración a `flavor = "multi_thread"` se mantiene estable en corridas repetidas contra el fix de null-safety de la migración 012, que es una precondición del baseline `develop` (heredada de WU1), no algo que esta unidad introduzca.
3. `AppBuilder::profile()` deliberadamente no se propaga a runtimes de entidad ya construidos y registrados vía `.entity()` — es un límite arquitectónico documentado (AD-5), no un hueco silencioso, y el doc comment del código lo declara palabra por palabra como lo exigió design.md.
4. La tabla `EXPECTED_PAIRS` de `schema_index_assertion.rs` no se extendió para cubrir el nuevo patrón de índice único parcial doble de `snapshots` de la migración 012, dejando un hueco de cobertura real (pero ya conocido, no bloqueante) en ese test de regresión.
5. El único diff de `cargo fmt --check` encontrado en `app/mod.rs` es anterior a PROD-013 por completo (confirmado presente en el commit baseline de `develop`), por lo que no debe atribuírsele erróneamente a este cambio durante el archive.
