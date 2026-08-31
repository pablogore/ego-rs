# Reporte de Verificación: PROD-013 — Endurecimiento de la Composición de Producción

> Compañero de revisión en español. Fuente canónica: `verify-report.md` (identificadores 1:1).

**Estado**: PASS
**Verificado contra**: `opsx/prod-013-wu7-architecture-docs` (acumula las 7 unidades de trabajo apiladas)
**Baseline en design.md**: `develop @ a740d34`

## Resumen Ejecutivo

Las 39 tareas de las 7 unidades de trabajo están funcionalmente completas y verificadas contra el código real (no contra reportes previos de apply). Los 6 comandos de gate (`cargo fmt --check`, `cargo check --workspace`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace` y el `run-suite` respaldado por Docker) pasan con **0 CRITICAL, 0 WARNING, 1 SUGGESTION preexistente/fuera de alcance** (una desviación de `cargo fmt` anterior a este cambio en `develop`). La verificación crítica #10 — si una composición `Profile::Production` puede seguir corriendo silenciosamente sobre almacenamiento volátil — no encontró ningún bypass: el gate está cerrado estructuralmente para reference-app (campo `profile` privado, dos constructores) y el único límite arquitectónico que sí existe (`AppBuilder::profile()` no se propaga a runtimes de entidad ya construidos) está documentado explícitamente en el código exactamente como lo exige AD-5 de `design.md`, y es el mismo riesgo residual que la propuesta (R-1) ya nombró y aceptó, no algo oculto.

## Completitud de Tareas (tasks.md)

| Fase | Tareas | Estado |
|---|---|---|
| 1 — `Profile` + predicado compartido | 1.1–1.5 | [x] todas, verificadas contra `profile.rs`, `error.rs`, re-export en `lib.rs` |
| 2 — Gate de `EntityRuntimeBuilder` + `try_build()` | 2.1–2.7 | [x] todas, verificadas contra `builder.rs` (tests + panic guard) |
| 3 — Gate del effect store | 3.1–3.7 | [x] todas, verificadas contra `service-sdk/runtime/builder.rs`, `app/mod.rs` |
| 4 — Cableado de `EntityEventStores` | 4.1–4.10 | [x] todas, verificadas contra `reference-app/src/lib.rs` |
| 5 — Fix de flavor de runtime Postgres | 5.1–5.3 | [x] todas, verificadas: 7 funciones de test en 4 archivos migradas a `flavor = "multi_thread"`; migración 012 presente y aplicada |
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
5. **`crates/persistence/src/postgres/snapshot.rs` + migración 012** — ambos existen; `IS NOT DISTINCT FROM` usado en ambos `SELECT`, `INSERT ... ON CONFLICT` ramificado tenant/systemwide apuntando cada uno a su propio índice parcial (`ux_snapshots_identity_tenant`, `ux_snapshots_identity_systemwide`). Confirmado.
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
2. La tabla `EXPECTED_PAIRS` de `integration-tests/tests/infrastructure/schema_index_assertion.rs` (que cubre `events`, `operation_reservations`, `operation_receipts`) **no** incluye la tabla `snapshots`, aunque la migración 012 (de este cambio) le aplica el mismo patrón de índice único parcial doble. Es un hueco de cobertura real en ese test de assertion, ya conocido y documentado como deuda aceptada según el brief del orquestador — no introducido como sorpresa por esta verificación, y explícitamente no bloqueante para este cambio.
3. `crates/persistence/src/postgres/repository.rs` (tabla `aggregates`) todavía usa el patrón no-null-safe `tenant_id = $2` en vez de `IS NOT DISTINCT FROM` — la misma clase de defecto que la migración 012 acaba de corregir para `snapshots`. Preexistente, fuera del alcance de PROD-013 (su conjunto de capacidades está fijado en exactamente tres: event/snapshot/effect store — D-3), y ya documentado como deuda de seguimiento aceptada según el brief del orquestador.
4. AD-11 (Approach C, la alternativa de invertir el default) permanece evaluado-y-diferido según D-7/OOS-5, tal como registra design.md; no existe tarea de implementación para ello y no se esperaba ninguna. No es un hueco — es un diferimiento deliberado y documentado.

## Veredicto

**PASS.** Las 39 tareas están funcionalmente completas y coinciden exactamente con el código tal como lo describen specs y design (incluyendo todas las correcciones de evidencia EC-1/EC-2 y las decisiones confirmadas AD-7/AD-8/AD-9). Los 5 gates solicitados están en verde (el único diff de `cargo fmt --check` está confirmado como preexistente, no relacionado con este cambio). No existe ningún gate de read-side, real ni pseudo (restricción dura D-4/OOS-2 honrada). El bypass motivador está cerrado estructuralmente para reference-app, y el único límite arquitectónico restante en el framework general es intencional, está documentado exactamente como lo exigió design.md, y coincide con el riesgo residual que la propia propuesta ya nombró y aceptó (R-1) en vez de dejarlo abierto en silencio.

**Próxima fase recomendada**: `sdd-archive`.

## Aprendizajes Clave

1. El campo privado `profile` de `EntityEventStores` con exactamente dos constructores cierra R-1 estructuralmente para reference-app — más fuerte que la garantía de "una llamada más un check de regresión" que la propuesta pedía originalmente.
2. El resultado exacto de la suite respaldada por Docker (43 passed / 0 failed / 1 ignored) coincide precisamente con el número de estabilización que design.md reporta en la tarea 5.3, confirmando que tanto la migración a `flavor = "multi_thread"` como el fix de null-safety de la migración 012 se mantienen estables en corridas repetidas.
3. `AppBuilder::profile()` deliberadamente no se propaga a runtimes de entidad ya construidos y registrados vía `.entity()` — es un límite arquitectónico documentado (AD-5), no un hueco silencioso, y el doc comment del código lo declara palabra por palabra como lo exigió design.md.
4. La tabla `EXPECTED_PAIRS` de `schema_index_assertion.rs` no se extendió para cubrir el nuevo patrón de índice único parcial doble de `snapshots` de la migración 012, dejando un hueco de cobertura real (pero ya conocido, no bloqueante) en ese test de regresión.
5. El único diff de `cargo fmt --check` encontrado en `app/mod.rs` es anterior a PROD-013 por completo (confirmado presente en el commit baseline de `develop`), por lo que no debe atribuírsele erróneamente a este cambio durante el archive.

---

## Verificación del Cierre de AD-12 (re-verify, WU8, `opsx/prod-013-wu8-durability-capability-check`)

**Contexto**: esta sección complementa el PASS anterior. No repite los 10 checks ya confirmados
allí — ese código no cambió. Verifica únicamente el trabajo nuevo de WU8: el fix de AD-12, el
hueco configuración-vs-durabilidad que `/code-review` encontró después del PASS previo de
`sdd-verify`, más el fix complementario de dedup en la migración 012. Base de esta corrida:
`opsx/prod-013-wu8-durability-capability-check`, que apila las 8 work units (7 previas + WU8)
sobre `develop @ a740d34`. Commits inspeccionados: `e0fa699`..`057470e` (12 commits, incluyendo
la cadena de WU8 `e503a30`→`057470e`).

### 1. El hueco es real y está cerrado — confirmado por lectura directa del código fuente, no confiando en el reporte de apply

- `crates/domain/src/persistence/event_store.rs:112-124` — `EventStore::is_durable(&self) -> bool { false }`, método default con doc comment que nombra AD-12. Confirmado literalmente.
- `crates/domain/src/persistence/snapshot.rs:39-49` — `Snapshot::is_durable(&self) -> bool { false }`, mismo patrón, misma referencia a AD-12. Confirmado literalmente.
- `crates/persistence/src/postgres/event_store.rs:422-425` — sobreescribe a `true` ("A committed append survives process death"). Confirmado.
- `crates/persistence/src/postgres/snapshot.rs:152-155` — sobreescribe a `true` ("A committed snapshot survives process death"). Confirmado.
- `crates/persistent-entity/src/builder.rs::validate_persistence()` (líneas 290-305) — ambas llamadas a `require_configured` ahora pasan `self.event_store.as_ref().is_some_and(|s| s.is_durable())` y `self.snapshot_store.as_ref().is_some_and(|s| s.lock().is_durable())`, no `.is_some()`. Confirmado byte por byte contra el diff prescrito en design.md AD-12.
- `crates/service-sdk/src/runtime/builder.rs::validate_persistence_profile()` (líneas 777-802) — chequea `self.effect_state_store.as_ref().is_some_and(|s| s.capabilities().durable)`, reutilizando el `EffectStoreCapabilities` existente de PROD-002 en vez de agregar un nuevo método de trait, exactamente como especifica AD-12. Confirmado.

**El escenario motivador exacto ahora es rechazado, ejercitado por un test real, no por un mock que simula el gate:**

```rust
EntityRuntimeBuilder::<TestEvent>::new()
    .profile(Profile::Production)
    .with_event_store(Arc::new(InMemoryEventStore::new()))
    .with_snapshot_store(Arc::new(Mutex::new(DurableStubSnapshotStore)))
    .try_build()
```

es afirmado `Err` por `try_build_rejects_explicit_in_memory_event_store_under_production`
(`crates/persistent-entity/src/builder.rs:763-782`), y el caso simétrico para el snapshot store
por `try_build_rejects_explicit_in_memory_snapshot_store_under_production` (líneas 784-806).
Ambos llaman a la cadena real `EntityRuntimeBuilder`/`require_configured`/`is_durable` de punta a
punta — ningún test double reemplaza al gate en sí, solo a las implementaciones de store bajo
prueba (`DurableStubEventStore`/`DurableStubSnapshotStore`, que existen únicamente para aislar un
campo a la vez, según la instrucción del propio design.md). Ejecutados explícitamente:

```
cargo test -p ego-persistent-entity try_build_rejects_explicit_in_memory_event_store_under_production
cargo test -p ego-persistent-entity try_build_rejects_explicit_in_memory_snapshot_store_under_production
```
→ ambos `ok`.

El equivalente para el effect store es
`validate_persistence_profile_rejects_explicit_in_memory_effect_store_when_executor_registered`
(`crates/service-sdk/src/runtime/builder.rs:3705-3729`), que afirma
`RuntimeError::PersistenceNotConfigured` para `Profile::Production` + un ejecutor registrado + un
`InMemoryEffectStore` explícito. Confirmado del mismo modo: llamada real a
`RuntimeBuilder::try_build()`, sin validador mockeado.

**Regresión que el propio fix requirió y reveló** (tarea 9.6): dos de los 39 tests previos
(`try_build_rejects_missing_snapshot_store_under_production` y la mitad "event-only" de
`try_build_rejects_partial_configuration_under_production`) habían cableado incidentalmente un
`InMemoryEventStore` explícito para aislar la afirmación del snapshot store; bajo la nueva regla de
durabilidad, ese event store en memoria es rechazado primero (el event store se chequea antes del
snapshot store, según el ordenamiento de AD-3), lo que habría invertido el mensaje de error
esperado. Confirmado el fix: ambos ahora usan un `DurableStubEventStore` local al test en su lugar,
y la afirmación original de cada test permanece sin cambios. Es un efecto secundario honesto y
revelado de endurecer la regla, no un debilitamiento silencioso de la cobertura.

### 2. Paso de dedup de la migración 012 — leído y sólido

`crates/persistence/src/postgres/migrations/012_fix_snapshots_tenant_null_uniqueness.sql` ahora
abre con un bloque `DELETE FROM snapshots ... USING (SELECT id, ROW_NUMBER() OVER (PARTITION BY
aggregate_id ORDER BY version DESC, created_at DESC, id DESC) ...) WHERE tenant_id IS NULL AND
rank > 1`, **antes** de `DROP INDEX IF EXISTS idx_snapshots_aggregate` y de las dos nuevas
sentencias `CREATE UNIQUE INDEX ... ux_snapshots_identity_{tenant,systemwide}`. El orden es
correcto: el dedup corre primero, por lo que un despliegue con filas duplicadas de tenant NULL
preexistentes (el defecto exacto que esta migración corrige) ya no falla directamente en
`CREATE UNIQUE INDEX`. El dedup conserva la fila con `version` más alto por `aggregate_id`
(desempate por `created_at DESC, id DESC`), lo que coincide con el propio `ORDER BY version DESC
LIMIT 1` de `load_snapshot` — por lo que ningún comportamiento observable cambia para ningún
llamador que pase por `Snapshot::load_snapshot`. El alcance está correctamente restringido a filas
con `tenant_id IS NULL`: el índice viejo ya garantizaba unicidad para tenants no nulos, por lo que
ninguna fila no nula corre riesgo de ser un duplicado. Esto coincide exactamente con la descripción
del fix complementario de AD-12 en design.md.

### 3. Gates re-ejecutados sobre el estado final de la rama (las 8 work units)

| Comando | Exit | Resultado |
|---|---|---|
| `cargo fmt --check` | 1 | Mismo diff preexistente único en `crates/service-sdk/src/app/mod.rs:275` (line-wrap de `record_app_started`) que en la corrida de verify previa — confirmado que sigue presente en el baseline de `develop`, sin tocar por ningún commit de PROD-013 incluyendo la cadena de WU8. SUGGESTION sin cambios, no es una regresión. |
| `cargo check --workspace` | 0 | Limpio. |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 | Limpio, cero warnings. |
| `cargo test --workspace` | 0 | 137 bloques de resultado de test (unit + doc-tests en todos los crates), todos `0 failed`. Sin regresión por el diff de WU8. |
| Docker `run-suite` (`DOCKER_HOST=unix:///Users/pablogore/.colima/default/docker.sock`) | 0 | 43 passed, 0 failed, 1 ignored (la misma exclusión Tier-1 preexistente/documentada) — conteo idéntico byte por byte a la corrida de verify previa, confirmando que la adición de dedup de la migración 012 no cambió nada observable para una base de datos fresca y que WU8 no introdujo ninguna regresión de infraestructura. |

### 4. Las Fases 1-8 no se vieron afectadas por WU8

Reconfirmado por la corrida completa de la suite arriba (0 fallos en ningún lado) más inspección
directa del diff: los commits de WU8 (`e503a30`..`057470e`) solo tocan
`crates/domain/src/persistence/{event_store,snapshot}.rs` (nuevo método default),
`crates/persistence/src/postgres/{event_store,snapshot}.rs` (override),
`crates/persistent-entity/src/builder.rs` (call-site + tests nuevos/ajustados),
`crates/service-sdk/src/runtime/builder.rs` (call-site + tests nuevos), y el archivo SQL de la
migración. Ningún archivo del diff de las Fases 1-8 (`e0fa699`..`3b1da9c`) fue tocado más allá de
estos mismos archivos en los mismos call sites que el design ya describía cambiando. Los tests de
las 39 tareas previamente verdes siguen todos pasando (confirmado vía la corrida completa arriba y
la contabilización explícita de la tarea 9.6 de los dos tests que necesitaron ajuste interno sin
debilitar sus afirmaciones).

### Veredicto de esta re-verificación

**PASS.** AD-12 está genuinamente cerrado: el gate ahora chequea `is_durable()`/
`capabilities().durable`, no mera presencia, en los tres call sites, verificado contra el código
fuente real y ejercitado por tests que llaman a la cadena real del builder de punta a punta en vez
de un sustituto del gate. El paso de dedup de la migración 012 está correctamente ordenado y
preserva el comportamiento. Los 5 gates están en verde sobre el estado final de la rama con las 8
work units, con resultados idénticos a la corrida de verify previa salvo por los tests nuevos de
WU8 mismos. Ningún CRITICAL, WARNING o SUGGESTION nuevo más allá del único diff preexistente de
`cargo fmt` ya registrado y re-confirmado como no relacionado con este cambio.

**Próxima fase recomendada**: `sdd-archive` — una vez que el arquitecto mergee la cadena de PRs,
el cambio completo de 8 work units queda listo para cerrarse.

## Aprendizajes Clave (re-verify WU8)

1. El patrón de fix de AD-12 reutilizó el `EffectStoreCapabilities.durable` existente de PROD-002
   para el effect store y agregó un método default mínimo equivalente `is_durable()` a
   `EventStore`/`Snapshot`, en vez de inventar una nueva estructura de capacidades, manteniendo el
   fix proporcional a un único booleano por trait.
2. Endurecer el argumento booleano de `require_configured` de presencia a durabilidad invirtió
   silenciosamente el mensaje de error esperado de dos tests preexistentes, porque habían cableado
   incidentalmente un event store en memoria para aislar una afirmación del snapshot store —
   revelado y corregido con un stub durable de test en vez de debilitar la afirmación.
3. El paso de deduplicación de la migración 012 debe correr antes de `DROP INDEX`/
   `CREATE UNIQUE INDEX`, y debe restringirse a filas con `tenant_id IS NULL`, ya que el índice no
   nulo preexistente ya garantizaba que no hubiera duplicados ahí.
4. Que los conteos exactos de passed/failed/ignored de la suite respaldada por Docker se mantengan
   idénticos entre las corridas de verify de WU7 y WU8 (43/0/1) es en sí mismo evidencia de que
   WU8 no cambió ningún comportamiento observable en tiempo de ejecución para una base de datos
   fresca.
