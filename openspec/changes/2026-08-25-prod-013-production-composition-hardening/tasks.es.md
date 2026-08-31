# Tareas: PROD-013 — Endurecimiento de la Composición de Producción

> Companion de revisión en español. Fuente de verdad canónica: `tasks.md` (identificadores 1:1).
> TDD estricto: cada tarea RED debe fallar por la razón correcta antes de iniciar su tarea GREEN pareja.
> AD-11 (Enfoque C) está confirmado en design.md como evaluado y diferido — este archivo no
> contiene ninguna tarea de implementación para él (8.3 es solo verificación).

## Pronóstico de Carga de Revisión

| Campo | Valor |
|-------|-------|
| Líneas cambiadas estimadas (un solo PR, todas las unidades) | ~800 líneas (adiciones+eliminaciones), ~15 archivos |
| Unidad de trabajo más grande (WU4, cableado de reference-app) | ~220 líneas / 4 archivos |
| Riesgo de presupuesto de 400 líneas (como un solo PR) | **Alto** |
| Riesgo de presupuesto de 400 líneas (por unidad encadenada) | Bajo (cada unidad se mantiene ≤ ~220 líneas) |
| PRs encadenados recomendados | Sí |
| División sugerida | PR 1 → PR 2 → PR 3 → PR 4 → PR 5 → PR 6 → PR 7 (PR 7 fusionable de forma independiente) |
| Estrategia de entrega | ask-on-risk |
| Estrategia de encadenamiento | **stacked-to-main** — confirmada por el arquitecto |

Decisión necesaria antes de aplicar: Resuelta
PRs encadenados recomendados: Sí
Estrategia de encadenamiento: stacked-to-main
Riesgo de presupuesto de 400 líneas: Alto (un solo PR) — mitigado encadenando en 7 unidades de trabajo

**Por qué AD-8/AD-9 (WU4) y la corrección del runtime de Postgres (WU5) son el punto de presión**,
dicho con honestidad y sin redondear hacia abajo: WU4 cablea dos instancias reales de
`PostgreSQLSnapshotStore` en `EntityEventStores::open`, agrega un campo `profile` privado + su
accesor, hace que `observed_entity_runtime` sea falible, y propaga `stores.profile()` a través de
la cadena `App::builder()` de `build_runtime_with` — implementación real en
`examples/reference-app/src/lib.rs` más ediciones de consecuencia en sus llamadores, no solo
código de validación. WU5 es una corrección de corrección (el pánico de `block_in_place` en un
runtime de un solo hilo), no relleno, y no puede omitirse una vez que WU4 se envía. Ninguna de
las dos es comprimible de forma segura sin ocultar riesgo.

### Unidades de Trabajo Sugeridas

| Unidad | Objetivo | PR probable | Comando de test focalizado | Arnés de runtime | Límite de rollback |
|------|------|-----------|----------------------|-----------------|-------------------|
| 1 | `Profile` + `require_configured` + `PersistenceCompositionError` en `persistent-entity` | PR 1 | `cargo test -p persistent-entity profile::tests` | N/A — solo tests unitarios, sin runtime real | Borrar `profile.rs`, la nueva variante de error y la línea `pub mod profile;`; nada depende de esto todavía |
| 2 | `EntityRuntimeBuilder::try_build()` + `build()` con gate por perfil | PR 2 | `cargo test -p persistent-entity try_build` | N/A — nivel unitario, sin runtime real | Revertir `.profile()`, `validate_persistence()`, `try_build()` y el guard de pánico de `build()`; `build()` vuelve byte-por-byte, ningún call site tocado |
| 3 | Gate del effect store en `RuntimeBuilder`/`AppBuilder` | PR 3 | `cargo test -p ego-service-sdk validate_persistence_profile` | `cargo test -p ego-service-sdk --test '*'` (propagación de CompositionError) | Revertir la nueva variante de `RuntimeError`, `validate_persistence_profile()`, `AppBuilder::profile()`; el fallback existente de `effect_store()` queda intacto |
| 4 | Cableado de `EntityEventStores` con perfil + `PostgreSQLSnapshotStore` durable en reference-app | PR 4 | `cargo test -p reference-app entity_event_stores` | Suite de Postgres con Docker: `EntityEventStores::open(pool)` y luego inspeccionar los stores cableados | Revertir los campos/constructores de `EntityEventStores` y las ediciones en `observed_entity_runtime`/`build_runtime_with`; la ruta `in_memory()` queda intacta ante un revert |
| 5 | Migrar los tests de integración Postgres en riesgo a `#[tokio::test(flavor = "multi_thread")]` | PR 5 | `cargo test -p integration-tests durable_entity_progress_postgres` (Docker) | Suite de Postgres con Docker — no existe sustituto para este riesgo | Revertir los cambios de flavor; seguro solo una vez revertido también el PR 4 (límite acoplado) |
| 6 | Guardas de regresión de AD-10 (Dev + Producción) | PR 6 | `cargo test -p reference-app --test production_profile_guard` | Suite de Postgres con Docker para la aserción en `durable_entity_progress_postgres.rs`; N/A para la guarda del lado Dev | Borrar el nuevo archivo de test y la aserción agregada; ningún código de producción tocado |
| 7 | Documentación de la regla de completitud de persistencia + límite con PROD-005 (IS-9/IS-10) | PR 7 | N/A — solo documentación | N/A — sin superficie ejecutable | Revertir el archivo de docs |

---

## Fase 1: Fundamento — `Profile` y el Predicado Compartido (AD-1, AD-2, AD-3)

- [x] 1.1 RED — `crates/persistent-entity/src/profile.rs`: test tabulado `require_configured_matrix` sobre {Dev, Production} × {configurado, no} (4 casos); falla al compilar.
- [x] 1.2 GREEN — crear `profile.rs`: enum `Profile` (`Dev` por defecto, `Production`), `require_configured(profile, configured, capability, fix)` (AD-1, AD-3).
- [x] 1.3 RED — `crates/persistent-entity/src/error.rs`: test que verifica que el mensaje de `PersistenceCompositionError::NotConfigured` nombra la capacidad Y la llamada de corrección (mismo patrón que `the_refusal_names_the_registration_and_the_opt_out` de PROD-012).
- [x] 1.4 GREEN — agregar `PersistenceCompositionError` (AD-2, `thiserror`) a `error.rs`.
- [x] 1.5 Cablear `pub mod profile;` en `persistent-entity/src/lib.rs`; agregar `pub use persistent_entity::profile::Profile;` en `service-sdk/src/runtime/mod.rs` (re-exportación de AD-1).

## Fase 2: Gate de `EntityRuntimeBuilder` + `try_build()` (AD-4, AD-6, AD-7)

- [x] 2.1 RED — `builder.rs`: `try_build_rejects_missing_event_store_under_production` nombra el event store + `.with_event_store()` (SC-1).
- [x] 2.2 RED — `try_build_rejects_missing_snapshot_store_under_production` (SC-2).
- [x] 2.3 RED — `try_build_rejects_partial_configuration_under_production` — event configurado / snapshot faltante Y el caso inverso, identificando el que falta en cada caso (SC-6, subsunción de AD-7, la asimetría del sitio 15 de EC-1).
- [x] 2.4 RED — `dev_profile_builds_on_nothing_configured` — `Profile::Dev`, nada configurado, sigue construyendo sobre in-memory (SC-5).
- [x] 2.5 RED — `build_panics_on_same_condition_try_build_refuses` — `Profile::Production`, capacidad faltante, `build()` entra en pánico con el mensaje de la negativa.
- [x] 2.6 GREEN — agregar `.profile()`, `validate_persistence()` (event store verificado antes que snapshot store, según el orden de AD-3), `try_build()` (validar antes de delegar); `build()` llama a `validate_persistence()` y entra en pánico en `Err` (AD-4/AD-6). No se agrega ningún puente `From<PersistenceCompositionError>` — la negativa de event/snapshot vuelve solo al host (AD-6).
- [x] 2.7 Verificación de migración SC-7 — correr `cargo build --workspace` y `cargo test --workspace`; confirmar que los 67 call sites existentes de `EntityRuntimeBuilder::new()` (25 archivos, re-verificados en design.md) compilan y pasan sin **ninguna edición de código fuente**. El valor por defecto `Profile::Dev` es lo que hace esto cierto; cualquier falla aquí es una desviación de diseño para señalar, no para parchear en silencio.

## Fase 3: Gate del Effect Store en `RuntimeBuilder`/`AppBuilder` (AD-5)

- [x] 3.1 RED — `service-sdk/src/runtime/builder.rs`: `validate_persistence_profile_rejects_missing_effect_store_when_executor_registered` (SC-3).
- [x] 3.2 RED — `validate_persistence_profile_ok_when_no_executor_registered` (gate condicional de EC-2 — sin executor no se construye nada, nada es volátil).
- [x] 3.3 RED — `build_and_try_build_agree_on_persistence_profile_validation`, siguiendo el test existente de acuerdo entre build/try_build para idempotencia.
- [x] 3.4 GREEN — agregar el campo `profile` + `.profile()` a `RuntimeBuilder`; agregar `validate_persistence_profile()` (AD-5), llamado desde `build()` y `try_build()` en el mismo lugar que `validate_idempotency()`; verifica solo `effect_state_store` (no `effect_dedup_store` — ambos siempre se configuran juntos mediante `with_effect_store`, según el `debug_assert_eq!` existente).
- [x] 3.5 GREEN — agregar `RuntimeError::PersistenceNotConfigured(#[from] PersistenceCompositionError)` en `runtime_builder.rs`.
- [x] 3.6 GREEN — agregar un `AppBuilder::profile()` delgado en `app/mod.rs`, siguiendo la forma de delegación de `effect_store()`; el comentario de documentación DEBE indicar que no se propaga a los entity runtimes ya construidos (`AppBuilder::entity()` recibe un `Arc<EntityRuntime<E>>` terminado — ese gate ya corrió).
- [x] 3.7 RED+GREEN — test de integración en `crates/service-sdk/tests/`: la negativa del effect store se propaga como `CompositionError::Validation` a través de `AppBuilder::build()`.

## Fase 4: Reference App — Perfil de `EntityEventStores` + Cableado del Snapshot Durable (AD-8, AD-9)

Línea base actual (verificada en `examples/reference-app/src/lib.rs`): `EntityEventStores` solo
tiene los campos de event store `org`/`user`; `observed_entity_runtime` (línea 488) no recibe
snapshot store ni perfil y llama a `.build()`; `compose_entity_runtimes` (línea 452) y
`build_runtime_with` (línea 567, que llama a `observed_entity_runtime` directamente en las líneas
649/654) necesitan actualizarse ambos.

- [x] 4.1 RED — test: `EntityEventStores::in_memory().profile() == Profile::Dev`.
- [x] 4.2 RED (integración, Docker) — test: `EntityEventStores::open(pool).await?.profile() == Profile::Production`.
- [x] 4.3 GREEN — agregar el campo privado `profile: Profile` + `pub fn profile(&self) -> Profile` a `EntityEventStores`; `in_memory()` fija `Profile::Dev`, `open()` fija `Profile::Production` (AD-8).
- [x] 4.4 RED — test: los snapshot stores de `EntityEventStores::open(pool)` están respaldados por `PostgreSQLSnapshotStore`, verificado por comportamiento (por ejemplo, un snapshot escrito sobrevive a una lectura fresca contra el mismo pool), no solo por chequeo de tipo.
- [x] 4.5 GREEN — agregar los campos `org_snapshot`/`user_snapshot: Arc<Mutex<dyn Snapshot + Send>>`; `open(pool)` construye **dos instancias tipadas** de `PostgreSQLSnapshotStore` sobre el pool compartido (no un `Arc` compartido — mismo razonamiento que ya existe para las instancias tipadas por agregado en `EntityEventStores`); `in_memory()` construye dos `InMemorySnapshotStore` (IS-13).
- [x] 4.6 GREEN — `observed_entity_runtime` (línea 488) recibe un parámetro de snapshot store y llama a `EntityRuntimeBuilder::try_build()`, devolviendo `Result` (consecuencia de AD-8).
- [x] 4.7 GREEN — actualizar ambos call sites: `compose_entity_runtimes` (línea 452, llamadas en 464/469) y `build_runtime_with` (línea 567, llamadas en 649/654) para pasar el snapshot store correspondiente y propagar el `Result`.
- [x] 4.8 Tarea de decisión — `compose_entity_runtimes` se queda en `.build()` (infalible), porque el campo de perfil es privado y `open()` siempre suministra todos los stores, así que ninguna entrada construible puede hacer que se rechace (el "las tareas deben elegir una y decir cuál" de AD-8); documentar esta elección en un comentario de código que cite AD-8.
- [x] 4.9 GREEN — en `build_runtime_with`, capturar `let profile = stores.profile();` **antes** de que `stores.org`/`stores.user`/los campos de snapshot se muevan hacia las llamadas a `observed_entity_runtime` (líneas 649/654), y luego llamar `.profile(profile)` en la cadena `App::builder()` (línea 683) en lugar de un literal fijo.
- [x] 4.10 Corregir call sites de consecuencia — actualizar `metrics_reach_one_backend.rs:209` y cualquier otro llamador de `compose_entity_runtimes`/`observed_entity_runtime` cuya firma cambió, sin ningún cambio de comportamiento en la ruta `Profile::Dev`.

## Fase 5: Riesgo de `block_in_place` / Flavor de Runtime en Postgres (mina de AD-9)

- [ ] 5.1 Auditoría — usando el umbral de ≥100 eventos por agregado (`PeriodicSnapshotStrategy::new(100)`), identificar todos los tests de integración Postgres que ahora pueden disparar un `PostgreSQLSnapshotStore::save_snapshot` real (que llama a `tokio::task::block_in_place`, con pánico en un runtime de un solo hilo). Empezar por los cuatro sitios nombrados: `durable_entity_progress_postgres.rs:187,233,289,374`. Revisar también `single_aggregate_crash_recovery_postgres.rs`, `dual_aggregate_crash_recovery_postgres.rs`, `concurrent_replicas_postgres.rs`.
- [ ] 5.2 GREEN — migrar cada test encontrado en 5.1 que pueda cruzar el umbral, de `#[tokio::test]` simple a `#[tokio::test(flavor = "multi_thread")]`; confirmar que la feature `rt-multi-thread` de `tokio` está habilitada en el crate correspondiente (agregarla en `Cargo.toml` si falta).
- [ ] 5.3 Verificación (requiere Docker) — correr los tests migrados y confirmar que no hay pánico de `block_in_place`; confirmar que los tests que se quedan bajo el umbral no se ven afectados al seguir en un solo hilo.

## Fase 6: Guardas de Regresión de AD-10

- [ ] 6.1 RED+GREEN — nuevo `examples/reference-app/tests/production_profile_guard.rs`: verifica que `EntityEventStores::in_memory().profile() == Profile::Dev` Y que `build_runtime_with` sobre stores in-memory sigue construyendo (guarda del lado Dev, SC-5 en la raíz de composición).
- [ ] 6.2 RED+GREEN — agregar una aserción en el `integration-tests/tests/infrastructure/durable_entity_progress_postgres.rs` existente, justo después de su llamada existente a `EntityEventStores::open()` (`:94`): `assert_eq!(stores.profile(), Profile::Production);` (guarda del lado Producción).

## Fase 7: Documentación (IS-9, IS-10)

- [ ] 7.1 Documentar la regla de completitud de persistencia textualmente (sección de Principio de Arquitectura de la propuesta) como guía prospectiva; indicar explícitamente que PostgreSQL no está en violación hoy (SC-10).
- [ ] 7.2 Documentar el límite con PROD-005: esta spec rechaza el bootstrap en sí antes de que nada arranque; PROD-005 señala la salud de una aplicación que ya arrancó (SC-10).

## Fase 8: Verificación Final

- [ ] 8.1 Correr `cargo test --workspace`; confirmar cero fallas nuevas (SC-7); confirmar que los 67 call sites de `EntityRuntimeBuilder::new()` en 25 archivos compilan sin modificación.
- [ ] 8.2 Correr `cargo clippy --workspace -- -D warnings`; confirmar que ninguna función nueva supera complejidad ciclomática 10.
- [ ] 8.3 Solo verificación, sin implementación — confirmar que AD-11 (Enfoque C) sigue registrado como evaluado y diferido en design.md; no hay nada que implementar aquí.
