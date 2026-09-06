# Tareas: STOOLAP-RS-01 — Almacenes de Lado de Lectura Durables sobre Stoolap

> Compañero en español. Fuente canónica de la verdad: `tasks.md` (numeración 1:1).
> TDD está activo (`openspec/config.yaml`): todo comportamiento nuevo aterriza en RED antes que
> en GREEN. La división en PRs, las estimaciones de líneas y las rutas de archivo están fijadas
> por las secciones "Migration / Rollout" y "File Changes" de `design.md` — no se re-derivan aquí.

## Pronóstico de Carga de Revisión

| Campo | Valor |
|-------|-------|
| Líneas cambiadas estimadas | ~1240 en total — PR1 ~280, PR2 ~210, PR3 ~380, PR4 ~250, PR5 ~120 (design.md "Migration / Rollout") |
| Riesgo del presupuesto de 400 líneas | Medio — cada rebanada se pronostica por debajo de 400 con margen; PR3 es la más cercana, medir una vez escrita |
| Se recomiendan PRs encadenados | Sí |
| División sugerida | PR1 → PR2 → PR3 → PR4 → PR5, cadena de rama de característica |
| Estrategia de entrega | ask-on-risk |
| Estrategia de encadenamiento | feature-branch-chain — PR1 apunta a la rama tracker, cada PR posterior apunta a su predecesor (design.md) |

Decisión necesaria antes de aplicar: Sí
PRs encadenados recomendados: Sí
Estrategia de encadenamiento: feature-branch-chain
Riesgo del presupuesto de 400 líneas: Medio

### Unidades de Trabajo Sugeridas

| Unidad | Objetivo | PR probable | Comando de prueba enfocado | Arnés en tiempo de ejecución | Límite de rollback |
|------|------|-----------|----------------------|-----------------|-------------------|
| 1 | Almacén de offset + andamiaje del feature `read-side` | PR1 | `cargo test -p ego-persistence-stoolap --features read-side --test read_side_stores` | base de datos Stoolap real respaldada por archivo temporal, cierre/reapertura real | eliminar `src/read_side/` + la entrada del feature `read-side`; con el feature apagado, `cargo build`/`cargo test --workspace` quedan sin cambios |
| 2 | Almacén de dedup | PR2 | mismo binario, sección de dedup | igual, tabla de dedup | eliminar `dedup.rs` + su `pub mod`/`pub use`; revertir la sección de dedup de `tests/read_side_stores.rs` |
| 3 | Elevación AD-11 + construcción del almacén de claim + pruebas unitarias | PR3 | `cargo test -p ego-persistence-stoolap --features read-side` (pruebas unitarias de claim) + `cargo test -p ego-persistence-stoolap --features operation-reservation` (regresión) | `ego_testkit::TestClock` avanzado explícitamente, base de datos en archivo temporal, nunca una espera real | revertir la elevación AD-11 (restaurar las dos funciones en `operation/reservation.rs`); eliminar `claim.rs` |
| 4 | Carrera de concurrencia de claim + durabilidad de reapertura + pruebas de motor compartido | PR4 | mismo binario, secciones de concurrencia/reapertura/motor compartido, `#[tokio::test(flavor = "multi_thread")]` | tareas `tokio::spawn` concurrentes reales contra una única base de datos Stoolap real en archivo temporal | revertir solo los nuevos casos de prueba; PR1-3 siguen siendo válidos |
| 5 | Composición de producción + control negativo | PR5 | `cargo test -p ego-service-sdk --test read_side_progress_composition` | `App::builder()`/`try_build()` reales sobre una base de datos Stoolap real en archivo temporal bajo `Profile::Production` | revertir las adiciones de la prueba de composición + la palabra de la feature de dev-dependency; PR1-4 siguen siendo válidos |

## Fase 1: Almacén de Offset + Andamiaje del Feature `read-side` — PR1

- [x] 1.1 `crates/persistence-stoolap/Cargo.toml`: añadir `read-side = ["dep:tokio", "dep:async-trait", "dep:chrono", "dep:ego-domain"]` (AD-1, las cuatro dependencias ya declaradas, cero adiciones al manifiesto) + `[[test]] name = "read_side_stores" required-features = ["read-side"]`.
- [x] 1.2 Ejecutar `cargo tree -p ego-persistence-stoolap --features read-side -e normal` y registrar la salida: confirmar cero aristas de dependencia transitiva nuevas frente al árbol con el feature apagado (afirmación de proposal/design, aún no ejecutada en explore).
- [x] 1.3 Crear `src/read_side/mod.rs` (`pub mod offset; pub mod dedup; pub mod claim;`) con un doc de módulo que enuncia el alcance de concurrencia solo-mismo-proceso (design "Concurrency Scope"); activar `#[cfg(feature = "read-side")] pub mod read_side;` en `src/lib.rs` + `pub use` en la raíz del crate de los tres tipos de almacén (AD-2).
- [x] 1.4 RED `src/read_side/offset.rs`: prueba unitaria `read_offset_of_a_never_written_key_is_none` — falla al compilar, `StoolapOffsetStore` aún no existe.
- [x] 1.5 GREEN mismo archivo: `StoolapOffsetStore::open(path: &Path) -> Result<Self, OffsetStoreError>` vía `stoolap_common::dsn_for`; rechazar con `Fatal` si `dsn_declares_sync_full` es falso antes de `CREATE TABLE` (AD-9); `CREATE TABLE IF NOT EXISTS projection_offsets (... UNIQUE (projection_id, tag, tenant))` — `UNIQUE`, nunca `PRIMARY KEY` (AD-3); `run_blocking` privado vía `tokio::task::spawn_blocking`, nunca `block_in_place` (AD-10); `fn is_durable(&self) -> bool { dsn_declares_sync_full(self.db.dsn()) }` — nunca un `true` codificado (AD-9).
- [x] 1.6 RED mismo archivo: `a_write_is_isolated_to_its_key` — dos claves `(projection_id, tag, tenant)` distintas, cada lectura devuelve solo su propio valor.
- [x] 1.7 RED mismo archivo: `a_repeat_write_overwrites_without_ordering_enforcement` — `write_offset` dos veces para la clave idéntica, `read_offset` devuelve el valor recién escrito (last-write-wins, sin CAS).
- [x] 1.8 GREEN mismo archivo: implementar el escritura de tres pasos de `write_offset` — UPDATE-primero / INSERT `ON CONFLICT DO NOTHING` / re-UPDATE (AD-4); implementar `read_offset`.
- [x] 1.9 RED+GREEN `crates/persistence-stoolap/tests/read_side_stores.rs` (crear): prueba de reapertura de offset — escribir, **soltar cada handle del almacén para esa ruta**, `open()` el mismo archivo de nuevo, `read_offset` para la clave escrita devuelve el valor idéntico y una clave hermana nunca escrita sigue devolviendo `None` (spec "Offset And Dedup State Survive Close And Reopen"); `is_durable()` devuelve `true` solo una vez demostrado esto.
- [x] 1.10 Verificación: `cargo test -p ego-persistence-stoolap --features read-side --test read_side_stores` en verde; `cargo build --workspace` y `cargo test --workspace` con el feature apagado quedan sin cambios (criterio de éxito de spec/proposal).
- [x] 1.11 Verificación: `rg block_in_place crates/persistence-stoolap/src/read_side` no devuelve nada; revisión manual de `crates/persistence-stoolap/src/read_side` y `crates/persistence-stoolap/tests/read_side_stores.rs` confirma que no hay ninguna afirmación positiva de soporte multi-proceso, multi-nodo, Kubernetes o coordinación distribuida — la documentación explícita que declara estos modos como no soportados es requerida y está permitida, no prohibida (un grep de prohibición ciega de palabras marcaría incorrectamente esa documentación requerida).

## Fase 2: Almacén de Dedup — PR2

- [x] 2.1 RED `src/read_side/dedup.rs`: prueba unitaria `seen_of_an_unmarked_triple_is_false` — falla al compilar, `StoolapDedupStore` aún no existe.
- [x] 2.2 GREEN mismo archivo: `StoolapDedupStore::open(path: &Path) -> Result<Self, DedupStoreError>`; `CREATE TABLE IF NOT EXISTS projection_dedup (... UNIQUE (projection_id, tag, event_id))` — **sin** columna de tenant, coincidiendo con la identidad propia del puerto sin tenant (AD-3); el mismo patrón de `open()` cerrado por defecto / `is_durable()` real que offset (AD-9); su propio `run_blocking` privado (AD-10).
- [x] 2.3 RED mismo archivo: `mark_seen_is_idempotent` — repetir `mark_seen` para la tripleta idéntica tiene éxito sin error, `seen()` sigue devolviendo `true`.
- [x] 2.4 RED mismo archivo: `no_dedup_entry_is_ever_pruned` — una marca escrita hace arbitrariamente mucho tiempo (simulado por la ausencia de cualquier ruta de limpieza basada en tiempo) sigue devolviendo `true` desde `seen()`; sin columna `seen_at`, sin TTL, sin retención (Non-Goal de spec).
- [x] 2.5 RED mismo archivo: `the_same_event_id_under_a_different_projection_and_tag_is_independent` — aislamiento a través de la clave completa.
- [x] 2.6 GREEN mismo archivo: implementar `mark_seen` (`INSERT ... ON CONFLICT (projection_id, tag, event_id) DO NOTHING`, AD-4) y `seen` (`SELECT 1 ... LIMIT 1`, la presencia es la respuesta).
- [x] 2.7 RED+GREEN `tests/read_side_stores.rs`: prueba de reapertura de dedup — marcar como visto, **soltar cada handle del almacén para esa ruta**, reabrir el mismo archivo, `seen()` para la tripleta marcada sigue devolviendo `true` (spec "A dedup mark survives a close/reopen cycle").
- [x] 2.8 Verificación: `cargo test -p ego-persistence-stoolap --features read-side --test read_side_stores` en verde tanto para la sección de offset como la de dedup; repetir el grep de `block_in_place` y la revisión de no-afirmación-positiva-multi-proceso de 1.11 sobre `dedup.rs`.

## Fase 3: Elevación del Helper AD-11 + Construcción del Almacén de Claim + Pruebas Unitarias — PR3

- [x] 3.1 Elevar `token_for_storage`/`token_from_storage` desde `crates/persistence-stoolap/src/operation/reservation.rs:76-93` hacia `crates/persistence-stoolap/src/persistence/stoolap_common.rs` como `pub(crate)` (AD-11) — preserva el comportamiento, no se eleva a `pub(crate)` en el mismo lugar, porque `operation/reservation.rs` está detrás de su propio `#[cfg(feature = "operation-reservation")]` y una referencia en el mismo lugar entre features rompería bajo `--features read-side` solo; actualizar el import de `reservation.rs` a la ruta elevada.
- [x] 3.2 `cargo test -p ego-persistence-stoolap --features operation-reservation` en verde — confirma que la mudanza AD-11 preserva el comportamiento, sin regresión en el almacén de reservación ya enviado.
- [x] 3.3 Crear `src/read_side/claim.rs`: `CREATE TABLE IF NOT EXISTS projection_claims (... UNIQUE (projection_id, tag, tenant))` — `UNIQUE`, nunca `PRIMARY KEY`, sin `CHECK` (AD-3); un shim `to_claim_error(ReservationError) -> ClaimError` que refleja `crates/persistence/src/postgres/read_side_claim.rs:51-57` (AD-11).
- [x] 3.4 GREEN mismo archivo: `StoolapReadSideClaimStore::open(path: &Path, clock: Arc<dyn ego_domain::Clock>) -> Result<Self, ClaimError>` — `Clock` inyectado (AD-7); el mismo patrón de `open()` cerrado por defecto / `is_durable()` real (AD-9); su propio `run_blocking` privado (AD-10).
- [x] 3.5 RED mismo archivo: `try_claim_grants_a_fresh_claim_with_no_live_lease` — `Ok(Some(fence))` con `FencingToken::initial()`.
- [x] 3.6 RED mismo archivo: `a_live_claim_refuses_a_second_claimant` — `Ok(None)`, el fence del titular existente permanece válido.
- [x] 3.7 GREEN mismo archivo: implementar el CAS de dos sentencias de `try_claim` — `INSERT ... ON CONFLICT (projection_id, tag, tenant) DO NOTHING`, luego un `UPDATE` condicional que reverifica el `fencing_token` y `lease_until` de la fila viva (AD-5), el patrón ya probado de `StoolapOperationReservationStore::reserve()`, no el `DO UPDATE ... RETURNING` de una sola sentencia de Postgres.
- [x] 3.8 RED mismo archivo: `takeover_of_a_lapsed_lease_mints_a_strictly_greater_token` — `Ok(Some(fence))` con un `fencing_token` estrictamente mayor, el fence del titular caducado ya no verifica.
- [x] 3.9 GREEN mismo archivo: completar la rama de takeover del CAS de `try_claim` (AD-5 paso 4).
- [x] 3.10 RED mismo archivo: `fencing_exhaustion_is_reported_not_wrapped` — `FencingToken::next() == None` en un takeover surge como `ClaimError::FencingExhausted`, nunca un token envuelto o truncado.
- [x] 3.11 GREEN mismo archivo: conectar la verificación de agotamiento en `try_claim` antes del `UPDATE` de takeover.
- [x] 3.12 RED mismo archivo: `renew_and_release_reject_a_stale_or_lapsed_fence_without_mutating_state` — un fence que ya no coincide con el claim vivo, y por separado un fence cuyo lease ya caducó, ambos fallan con `StaleOwner`, el claim almacenado sin cambios.
- [x] 3.13 GREEN mismo archivo: implementar un `fn set_lease(&self, fence, new_lease_until) -> Result<(), ClaimError>` privado — una sentencia, `UPDATE ... WHERE claim_id AND owner_id AND fencing_token AND lease_until > $now` (AD-6); `renew` lo llama con el `lease_until` del llamador; `release` lo llama con `clock.now()` (nunca un `DELETE`).
- [x] 3.14 RED mismo archivo: `release_marks_the_claim_expired_not_deleted` — después de `release`, un `try_claim` posterior para el `claim_id` idéntico tiene éxito de inmediato, y la fila sigue existiendo con un lease expirado (el fencing token sin cambios por el propio release).
- [x] 3.15 GREEN: confirmar que 3.14 pasa contra el `set_lease` de 3.13 (sin código de producción adicional — que `release` establezca un `lease_until` ya expirado es todo el mecanismo).
- [x] 3.16 GREEN mismo archivo: clasificación de errores — un `stoolap::Error` crudo para el cual `stoolap_common::is_write_conflict` es `true` mapea a `ClaimError::Transient`, todo lo demás a `Fatal`; `affected == 0` en una mutación verificada por fence mapea a `StaleOwner`, nunca a `Transient` (AD-8).
- [x] 3.17 Documentar + señalar (sin código de producción): una línea de doc de módulo en `claim.rs` enuncia la lectura satisfacible de "la expiración del lease es computada por el llamador" según design AD-7 — el *límite del lease* (`lease_until`) siempre es del llamador, y el "ahora" propio del almacén proviene únicamente del `Clock` inyectado, nunca del tiempo del sistema ambiente; el almacén nunca lee `Utc::now()`/`SystemTime::now()`/`now()` de SQL. Registrar en la descripción del PR que la redacción de `spec.md`/`spec.es.md` de "Lease Expiry Is Always Caller-Computed" ("never on a clock read performed inside the store") es literalmente inimplementable contra la firma real de `try_claim` (sin parámetro `now`) y señalarlo a `sdd-verify`/un humano para un pase de aclaración de seguimiento — no reinterpretar en silencio sin este rastro documentado.
- [x] 3.18 Verificación: `cargo tree -p ego-persistence-stoolap --features read-side -e normal` sin cambios respecto a 1.2 (AD-11 solo mueve código, no añade dependencia); `cargo test -p ego-persistence-stoolap --features read-side` (pruebas unitarias de claim) en verde; `cargo test -p ego-persistence-stoolap --features operation-reservation` en verde (sin regresión); repetir el grep de `block_in_place` y la revisión de no-afirmación-positiva-multi-proceso sobre `claim.rs`.

## Fase 4: Carrera de Concurrencia de Claim + Durabilidad de Reapertura + Pruebas de Motor Compartido — PR4

- [x] 4.1 RED `tests/read_side_stores.rs`, `#[tokio::test(flavor = "multi_thread")]` + `tokio::spawn`: `concurrent_claimants_yield_exactly_one_winner` — varias tareas compiten por `try_claim` sobre un único `claim_id` fresco sin lease vivo existente, reflejando la forma de `tests/reservation_conformance.rs:192-240` en `two_concurrent_reserves...`.
- [x] 4.2 GREEN: confirmar que exactamente una tarea recibe `Ok(Some(fence))` y toda otra recibe `Ok(None)` o `ClaimError::Transient` (clasificado vía `is_write_conflict`, AD-8) — nunca un segundo `Ok(Some(fence))` para el mismo lease vivo.
- [x] 4.3 RED mismo archivo: `takeover_after_expiration_under_real_concurrency_mints_a_strictly_greater_token` — dos tareas concurrentes reales compiten por un takeover desde un único lease caducado.
- [x] 4.4 GREEN: confirmar que el perdedor resuelve a `Ok(None)` (un par tomó el control o el titular renovó en la ventana) o un `Transient` seguro de reintentar — nunca un tercer resultado, nunca dos concesiones.
- [x] 4.5 RED mismo archivo: `claim_state_survives_close_and_reopen` — mantener un claim bajo un fence válido, **soltar cada handle del almacén para esa ruta**, reabrir el mismo archivo; el `try_claim` de un titular diferente sobre el `claim_id` idéntico sigue devolviendo `Ok(None)` y el fence mantenido sigue verificando a través de `renew`; por separado, un fence liberado antes de soltar los handles reabre como reclamable de inmediato con un token estrictamente mayor en el siguiente takeover. El nombre de la prueba y el comentario del módulo enuncian explícitamente que esto es soltar-y-reabrir, **no** seguridad ante caída/pérdida de energía (spec "Claim Durability Is Drop-And-Reopen, Not Crash Recovery").
- [x] 4.6 GREEN: confirmar que 4.5 pasa; revisar el comentario de doc de la prueba y las aserciones para confirmar que no se implica ningún lenguaje de seguridad ante caídas en ningún lugar.
- [x] 4.7 RED+GREEN mismo archivo: `three_stores_at_one_path_share_one_engine` — los almacenes de offset, dedup y claim abiertos en una ruta idéntica observan todos la misma base de datos (la propiedad de motor único global-por-proceso-por-DSN de Stoolap), reflejando el precedente de `tests/reservation_conformance.rs:258-311`; re-probado aquí, no asumido (design "Concurrency Scope").
- [x] 4.8 Verificación: `cargo test -p ego-persistence-stoolap --features read-side --test read_side_stores` en verde; si es inestable bajo hilos paralelos por defecto, re-ejecutar con `--test-threads=1` (precedente S3) y registrar el hallazgo; el grep confirma que no hay `block_in_place`; la revisión manual confirma que no hay ninguna afirmación positiva de soporte multi-proceso/multi-nodo/Kubernetes/distribuido en ningún lugar de los nombres, comentarios o docs de las pruebas nuevas de este PR (la documentación de modos no soportados sigue siendo requerida y está permitida).

## Fase 5: Composición de Producción + Control Negativo — PR5

- [x] 5.1 `crates/service-sdk/Cargo.toml`: añadir `"read-side"` a la lista de features de dev-dependency existente de `ego-persistence-stoolap` (AD-13, una palabra).
- [x] 5.2 RED `crates/service-sdk/tests/read_side_progress_composition.rs` (extender, archivo existente): una composición real de `Profile::Production` sobre una base de datos Stoolap real respaldada por `tempfile::tempdir()` en disco, usando `StoolapOffsetStore`, `StoolapDedupStore` y `StoolapReadSideClaimStore` reales a través de `App::builder()` / `try_build()` — no solamente `is_durable()` sobre un almacén aislado (spec "A Real Profile::Production Composition Exercises The Gate").
- [x] 5.3 GREEN: confirmar que la composición durable de Stoolap se construye con éxito bajo la puerta sin modificar.
- [x] 5.4 RED mismo archivo: control negativo — la composición idéntica con exactamente un almacén intercambiado por el `VolatileOffsetStore` ya existente en el archivo (o un almacén de claim/dedup volátil equivalente) bajo `Profile::Production`.
- [x] 5.5 GREEN: confirmar que la misma puerta, sin modificar, rechaza la composición de control negativo como `CompositionError::Validation(RuntimeError::PersistenceNotConfigured(_))` — este PR no toca ningún código de la puerta.
- [x] 5.6 Verificar-sin-tocar: leer el Purpose, Requirements y Non-Goals de `openspec/specs/real-infrastructure-verification/spec.md`; confirmar que los cinco requisitos siguen siendo específicos de PostgreSQL y ninguno queda involucrado por este cambio (design AD-14); registrar la confirmación en la descripción del PR; hacer **cero** ediciones a ese archivo.
- [x] 5.7 Verificación: `cargo test -p ego-service-sdk --test read_side_progress_composition` en verde; `cargo test --workspace` con el feature `read-side` apagado en `persistence-stoolap` sin afectar; `cargo tree -p ego-service-sdk --features read-side -e normal` final (o verificación equivalente de todo el workspace) confirma cero dependencias transitivas nuevas introducidas de punta a punta por todo el cambio.

## Criterios de Aceptación Transversales (aplican a cada PR anterior, no enunciados una sola vez)

- Ninguna dependencia de PostgreSQL introducida en ningún lugar de este cambio.
- Ningún `block_in_place` en ningún lugar — solo `tokio::task::spawn_blocking` vía el `run_blocking()` propio de cada almacén.
- Ninguna afirmación positiva de soporte multi-proceso, multi-nodo, Kubernetes o coordinación distribuida en ningún doc, comentario o nombre de prueba enviado por este cambio. La documentación explícita que declara estos modos como no soportados es requerida y está permitida — este criterio prohíbe afirmaciones falsas de soporte, no las palabras en sí.
- El `is_durable()` de cada almacén reporta `true` solo cuando está respaldado por una conexión Stoolap real, cerrada por defecto, verificada con `sync=full` — nunca un literal codificado.
- `cargo tree` (o equivalente) confirma cero dependencias transitivas nuevas para el feature `read-side` — registrado en PR1 (1.2), reconfirmado en PR3 (3.18) y PR5 (5.7).

## Fuera de Alcance (reafirmado, no re-litigado)

Ninguna tarea anterior toca código de producción de `EventStore`, `Snapshot`, `Repository`, `EffectStateStore`, `EffectDedupStore` u `OperationReservationStore` (reutilizado solo como plantilla de patrón); ninguna tarea cambia un archivo de Postgres o cualquiera de los tres contratos de trait en `crates/persistence-api/src/read_side/`; ninguna tarea añade poda/TTL/retención de dedup ni compare-and-swap/monotonicidad de offset (Non-Goals de proposal, sin cambios).
