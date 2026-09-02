# Tareas: CORE-PERSIST-B — Adaptador de Persistencia en Memoria de Primera Clase

> Compañero en español. Fuente canónica / de referencia: `tasks.md` (identificadores 1:1).
> TDD estricto (`openspec/config.yaml` → `apply.tdd: true`): el RED de cada slice es un fallo de
> compilación que nombra una ruta que aún no existe (design AD-9) — un RED válido según
> `ego-rs-testing-tdd`. Este cambio no escribe comportamiento nuevo, por lo que ninguna tarea
> agrega una aserción de comportamiento; las aserciones que importan ya existen y deben seguir
> pasando **sin modificar**. El orden de slices es el obligatorio de design AD-9: S1
> (infraestructura) → S2 (store de reservas) → S3 (reference-app), cada uno compilando de forma
> independiente en todo el workspace antes de que empiece el siguiente.
>
> **Nota de trazabilidad OQ-2**: S2 hace que `InMemoryOperationReservationStore` sea alcanzable
> desde producción por primera vez (D-7, AD-8). El usuario AUTORIZÓ explícitamente este cambio de
> alcanzabilidad — no existe ninguna tarea adicional de aprobación; la Fase 9 lo registra como un
> hecho, no como una pregunta abierta.

## Pronóstico de Carga de Revisión

Conteos de líneas de fuente medidos (no estimados): `event_store.rs` 268, `read_side_store.rs`
225, `repository.rs` 69, `snapshot.rs` 49 (S1, 611 líneas en total) · `reservation.rs` 573, cuyo
bloque de store/`Record`/`RecordState` que se mueve es la mayoría (S2) · `store.rs` de
reference-app 413, cuyas dos estructuras que se mueven son un bloque de ~90 líneas (S3). Según el
riesgo R-4 de la propuesta y el propio planteamiento de design AD-9, la reubicación verbatim
cuenta el add+delete completo, no un diff resumido — CORE-PERSIST-A midió 1.600–2.000 líneas
crudas para una versión más pequeña, de dos crates, de esta misma forma.

| Campo | Valor |
|-------|-------|
| Líneas cambiadas estimadas | ~1.400–1.900 en total — S1 ~600–850 (4 archivos + esqueleto + test de identidad), S2 ~500–700 (el archivo único más grande, más los bordes de `Cargo.toml`/`lib.rs`), S3 ~250–350 (2 archivos + 2 ediciones de `mod.rs`) |
| Riesgo de presupuesto de 400 líneas | Alto para el total combinado; los slices individuales se acercan al presupuesto de 800 líneas, pero S1 y S2 también corren el riesgo de superarlo al contar los comentarios de documentación y los tests de identidad |
| Se recomiendan PRs encadenados | Sí |
| División sugerida | PR 1 (S1 — infraestructura) → PR 2 (S2 — store de reservas) → PR 3 (S3 — reference-app) |
| Estrategia de entrega | stacked-to-main, 3 PRs (decidida — reemplaza el single-pr del preflight de la sesión) |
| Estrategia de encadenamiento | stacked-to-main — coincide con el orden obligatorio de AD-9 y su reversibilidad por slice |

Decisión registrada: el usuario aceptó explícitamente cambiar single-pr por la cadena de 3 PRs a
continuación, dado que el presupuesto de 800 líneas se superaba con el total combinado (según el
riesgo R-4 de la propuesta y design AD-9). No se otorgó ninguna `size:exception` — la estrategia de
entrega misma cambió en su lugar.
Se recomiendan PRs encadenados: Sí
Estrategia de encadenamiento: stacked-to-main
Riesgo de presupuesto de 400 líneas: Alto

### Unidades de Trabajo Sugeridas

| Unidad | Objetivo | PR | Rama desde | Comando de test enfocado | Arnés de runtime | Límite de rollback |
|------|------|-----|----------------|----------------------|-----------------|-------------------|
| 1 | S1 — esqueleto del nuevo crate, `layers.toml`, 4 implementaciones de `ego-infrastructure` reubicadas y re-exportadas | PR 1 | `develop` | `cargo build -p ego-persistence-memory && cargo test -p ego-infrastructure` | N/A — reubicación puramente estructural, sin comportamiento nuevo que ejercitar (design Testing Strategy) | Eliminar `crates/persistence-memory/`, restaurar los 4 archivos + `in_memory/mod.rs`, revertir los cambios en `layers.toml`/`Cargo.toml` raíz/`infrastructure/Cargo.toml` |
| 2 | S2 — `InMemoryOperationReservationStore` separado de `ego-testkit`, se agregan los bordes `ego-domain`+`chrono` | PR 2 | PR 1 | `cargo build -p ego-persistence-memory && cargo test -p ego-testkit` | N/A — misma razón | Restaurar las declaraciones previas a la división en `reservation.rs`, eliminar `operation/reservation.rs`, revertir los dos cambios de `Cargo.toml`; el PR 1 sigue siendo válido |
| 3 | S3 — `InMemoryOffsetStore`/`InMemoryDedupStore` reubicados fuera de `examples/reference-app` | PR 3 | PR 2 | `cargo build -p reference-app && cargo test --workspace` | N/A — misma razón | Restaurar las dos declaraciones del ejemplo y sus dos sitios de import; los PR 1–2 permanecen válidos |

## Fase 1: Esqueleto del Crate y Puerta de Capas — S1 — PR 1

- [ ] 1.1 Crear `crates/persistence-memory/Cargo.toml` (paquete `ego-persistence-memory`): deps `ego-persistence-api` (path), `async-trait` (workspace), `serde_json`; dev-deps `tokio` (`macros`, `rt`), `chrono` — solo dev en S1, promovida a normal en S2 (AD-2, EC-4, EC-7).
- [ ] 1.2 Crear `crates/persistence-memory/src/lib.rs`: solo `pub mod persistence;` + `pub mod read_side;`, sin re-exports en la raíz del crate, sin `#![deny(missing_docs)]` (AD-3 Refinamientos 2–3).
- [ ] 1.3 Crear los esqueletos de `src/persistence/mod.rs` y `src/read_side/mod.rs` declarando sus submódulos de S1 (AD-3).
- [ ] 1.4 Agregar la entrada `"ego-persistence-memory" = "foundation"` a `layers.toml`. No abrir `xtask/src/layers.rs` (AD-1).
- [ ] 1.5 Agregar `"crates/persistence-memory",` a los miembros del workspace en el `Cargo.toml` raíz.
- [ ] 1.6 Agregar la dependencia de path `ego-persistence-memory` a `crates/infrastructure/Cargo.toml`.

## Fase 2: RED — Test de Identidad de Compatibilidad, S1 — PR 1

- [ ] 2.1 Crear `crates/infrastructure/tests/in_memory_reexport_identity.rs` con un testigo de identidad por cada fila de S1 de la matriz de compatibilidad restablecida: `InMemoryEventStore`, `InMemoryRepository`, `InMemorySnapshotStore`, `{InMemoryReadSideStore, paginate}` (los traits object-safe obtienen una coerción de identidad; `paginate` obtiene un test de igualdad de puntero a función, según AD-10). `InMemoryEventStoreUnitOfWork` no necesita ninguno — es privado, solo alcanzable vía `Box<dyn EventStoreUnitOfWork>`. Falla al compilar: ninguna de las rutas `ego_persistence_memory::…` existe todavía.

## Fase 3: GREEN — Reubicar las Cuatro Implementaciones de `ego-infrastructure` — S1 — PR 1

- [ ] 3.1 Mover `event_store.rs` verbatim a `src/persistence/event_store.rs`; reescribir sus 4 líneas `use ego_domain::…` a `ego_persistence_api::…` según la fila 1 de AD-4.
- [ ] 3.2 Mover `repository.rs` verbatim a `src/persistence/repository.rs`; reescritura de la fila 2 de AD-4.
- [ ] 3.3 Mover `snapshot.rs` verbatim a `src/persistence/snapshot.rs`; reescritura de la fila 3 de AD-4 (`use serde_json::Value;` sin cambios).
- [ ] 3.4 Mover `read_side_store.rs` verbatim — incluyendo su módulo `#[cfg(test)]` (EC-4) — a `src/read_side/store.rs` (renombrado del Refinamiento 1 de AD-3); reescritura de la fila 4 de AD-4.

## Fase 4: GREEN — Re-export en las Rutas Antiguas, S1 — PR 1

- [ ] 4.1 Reemplazar las 4 declaraciones `mod` de `crates/infrastructure/src/persistence/in_memory/mod.rs` por 4 líneas `pub use ego_persistence_memory::…` a nivel de ítem (AD-6); eliminar los 4 archivos de origen ahora vacíos; dejar el doc del módulo (`:1-5`) sin cambios.

## Fase 5: Verificación — S1 — PR 1

- [ ] 5.1 `cargo build -p ego-persistence-memory` funciona de forma independiente.
- [ ] 5.2 `cargo build --workspace` funciona; pone en verde los testigos de identidad de 2.1.
- [ ] 5.3 `cargo run -p xtask -- verify-layers` pasa: el nuevo crate está mapeado, sin edición de la matriz, sin ciclos (R11).
- [ ] 5.4 `cargo test --workspace` pasa; el conteo de aserciones del módulo de test reubicado de `read_side_store.rs` no cambia (R5).
- [ ] 5.5 Lectura de diff: `crates/infrastructure/tests/in_memory_event_store_conformance.rs`, `crates/infrastructure/tests/commit_publishes_atomically.rs`, `examples/reference-app/src/lib.rs:432-439` compilan con código fuente idéntico byte a byte (R9).
- [ ] 5.6 Puerta de diff cero semántico: comparar cada uno de los 4 archivos movidos, ruta antigua vs. nueva — idénticos salvo la ruta de módulo y las líneas de import enumeradas en AD-4 (R5, R2).

## Fase 6: RED — Test de Identidad de Compatibilidad, S2 — PR 2

- [x] 6.1 Crear `crates/testkit/tests/reservation_reexport_identity.rs` con un testigo de identidad para `InMemoryOperationReservationStore` en `ego_persistence_memory::operation::reservation::InMemoryOperationReservationStore`. Falla al compilar: la ruta aún no existe.

## Fase 7: GREEN — Dividir `reservation.rs` y Reubicar el Store de Reservas — S2 — PR 2

- [x] 7.1 Agregar `ego-domain` (path) a `crates/persistence-memory/Cargo.toml` y promover `chrono` de dev a dependencia normal — el único slice que amplía el borde de dependencias (AD-2, D-3).
- [x] 7.2 Agregar `pub mod operation;` a `src/lib.rs`; crear `src/operation/mod.rs`.
- [x] 7.3 Mover `RecordState`, `Record`, `InMemoryOperationReservationStore` y su `impl OperationReservationStore` verbatim desde `crates/testkit/src/reservation.rs` a `src/operation/reservation.rs`. Reescribir el import de once nombres de `operation::` a `ego_persistence_api::operation::reservation::{…}` (EC-1, fila 7 de AD-4); reescribir la ruta inline `fingerprint: ego_domain::operation::OperationFingerprint` a `ego_persistence_api::operation::key::OperationFingerprint` (EC-2, AD-5). `use ego_domain::Clock;` queda sin cambios — la única línea `ego_domain::` que sobrevive en el crate.
- [x] 7.4 Agregar la dependencia de path `ego-persistence-memory` a `crates/testkit/Cargo.toml`.

## Fase 8: GREEN — Re-export Dentro de `reservation.rs`; `TestClock` y los Tests se Quedan — S2 — PR 2

- [x] 8.1 — **ENMENDADO, EC-8**: Podar los imports de `crates/testkit/src/reservation.rs` a solo lo que necesitan `TestClock` y el test de conformidad que se queda (`std::sync::Arc`, `chrono::{TimeZone, Utc}`); agregar `pub use ego_persistence_memory::operation::reservation::InMemoryOperationReservationStore;` dentro del módulo, inmediatamente después (EC-3, AD-5). Dejar `TestClock`, su `impl Clock`, y `the_in_memory_reservation_store_conforms` idénticos byte a byte. `a_lock_wait_that_spans_expiry_rejects_the_lapsed_holder` NO se queda — bloquea directamente el campo `records`, ahora privado fuera del crate, así que se reubica (cuerpo idéntico byte a byte) a un nuevo `#[cfg(test)] mod tests` colocalizado en `crates/persistence-memory/src/operation/reservation.rs`, impulsado por un nuevo doble local `FixedClock` (design.md EC-8; decisión del usuario, misma postura que OQ-2 — "lo más arquitectónicamente claro").
- [x] 8.2 Confirmar que `crates/testkit/src/lib.rs:50` no necesita ninguna edición (EC-3).

## Fase 9: Verificación — S2 — PR 2

- [x] 9.1 `cargo build -p ego-persistence-memory` funciona de forma independiente con los nuevos bordes `ego-domain`/`chrono`.
- [x] 9.2 `cargo build --workspace` funciona; pone en verde el testigo de identidad de 6.1.
- [x] 9.3 — **ENMENDADO, EC-8**: `rg '^use ego_domain::|ego_domain::' crates/persistence-memory/src` devuelve exactamente una línea **no-test** (criterio 4 de AD-2); existe una segunda línea, protegida por `#[cfg(test)]`, para el `FixedClock` del test colocalizado.
- [x] 9.4 `cargo run -p xtask -- verify-layers` pasa con el borde `ego-domain` (`foundation → domain`, ya permitido, sin edición de la matriz).
- [x] 9.5 — **ENMENDADO, EC-8**: `cargo test --workspace` pasa; `the_in_memory_reservation_store_conforms` (que se queda en `ego-testkit`) y `a_lock_wait_that_spans_expiry_rejects_the_lapsed_holder` (reubicado en `ego-persistence-memory`) pasan ambos con el cuerpo sin modificar (D-8, R16). Confirmado: corrida completa de `cargo test --workspace`, cero líneas `FAILED`, ambos tests `... ok`.
- [x] 9.6 Lectura de diff: `git diff develop -- crates/transport/tests/operation_key_extractor.rs crates/service-sdk/tests/retention_worker_lifecycle.rs crates/service-sdk/tests/cross_tenant_reservation_isolation.rs` devuelve vacío — los tres compilan con código idéntico byte a byte (R9). Registrar en la descripción del PR: este slice hace que `InMemoryOperationReservationStore` sea alcanzable desde producción, según D-7/AD-8, ya AUTORIZADO — sin tarea adicional de aprobación.
- [x] 9.7 Puerta de diff cero semántico para el cuerpo movido del store de reservas (R5, R2). Confirmado: el `diff` de `RecordState`/`Record`/`InMemoryOperationReservationStore`/`impl OperationReservationStore` entre el cuerpo pre-mudanza (`develop:crates/testkit/src/reservation.rs`) y el post-mudanza (`crates/persistence-memory/src/operation/reservation.rs`) muestra exactamente una línea cambiada — el tipo del campo `fingerprint` (`ego_domain::operation::OperationFingerprint` → `ego_persistence_api::operation::key::OperationFingerprint`), ya documentada como la reescritura deliberada de EC-2/AD-5. Cero otro cambio semántico.

## Fase 10: RED — Reapuntar los Imports de Reference-App Antes de que Existan las Nuevas Rutas — S3 — PR 3

- [ ] 10.1 En `examples/reference-app/src/read_side/store.rs`, eliminar las declaraciones de `InMemoryOffsetStore`/`InMemoryDedupStore` y reemplazar los imports relevantes del archivo por `use ego_persistence_memory::read_side::{dedup::InMemoryDedupStore, offset::InMemoryOffsetStore};` (AD-7). `cargo build -p reference-app` falla: las rutas aún no existen en `ego-persistence-memory`.
- [ ] 10.2 Agregar la dependencia de path `ego-persistence-memory` a `examples/reference-app/Cargo.toml`.

## Fase 11: GREEN — Reubicar los Dos Stores de Read-Side — S3 — PR 3

- [ ] 11.1 Agregar `pub mod offset;` y `pub mod dedup;` a `src/read_side/mod.rs`.
- [ ] 11.2 Crear `src/read_side/offset.rs`: `InMemoryOffsetStore` + `OffsetKey` movidos verbatim desde `store.rs` de reference-app; reescribir su import a `ego_persistence_api::read_side::{offset::{Offset, OffsetStore, OffsetStoreError}, event_tag::EventTag}` (fila 5 de AD-4).
- [ ] 11.3 Crear `src/read_side/dedup.rs`: `InMemoryDedupStore` + `DedupKey` movidos verbatim; misma forma de reescritura para `dedup`/`event_tag` (fila 6 de AD-4).
- [ ] 11.4 Actualizar `examples/reference-app/src/read_side/mod.rs:36-39`: reemplazar los dos nombres eliminados por `pub use ego_persistence_memory::read_side::{dedup::InMemoryDedupStore, offset::InMemoryOffsetStore};`; mantener `pub use store::{FakeDurableDedupStore, FakeDurableOffsetStore, ReadSideSink, SharedReadSideStore};` (AD-7).

## Fase 12: Verificación — S3 — PR 3

- [ ] 12.1 `cargo build -p reference-app` funciona — pone en verde el RED de 10.1.
- [ ] 12.2 `cargo build --workspace` funciona.
- [ ] 12.3 `cargo test --workspace` pasa; el propio módulo `#[cfg(test)]` del ejemplo (`store.rs:309+`) sigue ejercitando `SharedReadSideStore`, `ReadSideSink`, y ambos wrappers `FakeDurable*` sin modificar (NG-8, R3).
- [ ] 12.4 Lectura de diff: `FakeDurableOffsetStore`/`FakeDurableDedupStore` permanecen idénticos byte a byte, declarados solo en el ejemplo; `OffsetKey`/`DedupKey` se movieron junto con sus estructuras (EC-5).
- [ ] 12.5 Puerta de diff cero semántico para ambos stores de read-side reubicados.

## Fase 13: Verificación del Cambio Completo y Auditoría de Diff — PR 3

- [ ] 13.1 `cargo run -p xtask -- verify-layers` pasa de extremo a extremo: el nuevo crate está mapeado, cero violaciones, matriz sin tocar (R11).
- [ ] 13.2 Lectura de diff a través de los tres PRs: cero archivos SQL/migración/`crates/persistence/` (R13); `crates/runtime/` y `crates/effect-store/` idénticos byte a byte, `InMemoryEffectStore` y sus tres puertos sin tocar (R12); `crates/persistence-api/src/**` idéntico byte a byte (R15); `crates/persistent-entity/` sin tocar, ambos duplicados siguen bifurcados (R17, EC-6).
- [ ] 13.3 Confirmar que el conteo de bloques `impl <Puerto> for` en todo el workspace por puerto movido no cambia (R2, R10); las únicas declaraciones no canónicas que sobreviven son los dos duplicados nombrados de `persistent-entity` y los fakes de test declarados.
- [ ] 13.4 Confirmar que `ProjectionStateStore` sigue con cero implementaciones, sin stub ni `todo!()` en ninguna parte del nuevo crate (R4).
- [ ] 13.5 Confirmar que `presence_alone_is_not_durability` y ambos tests `try_build_rejects_explicit_in_memory_*` (`persistent-entity/src/builder.rs:768,793`, `profile.rs:99-117`) pasan sin modificar (R6).
- [ ] 13.6 Confirmar que `crates/persistence-memory/Cargo.toml` nombra exactamente `ego-persistence-api` y `ego-domain` como dependencias de path del workspace (R11); confirmar que ningún token de `sqlx`/Postgres/Stoolap/HTTP/Kafka aparece en ninguna parte bajo `crates/persistence-memory/` (R7).

## Diferido / Fuera de Alcance (deuda nombrada, no tareas)

- **KD-1** — `ProjectionStateStore` permanece en cero implementaciones. Ninguna tarea la implementa (NG-10, D-12, R4).
- **KD-5 → F-5** — `InMemorySnapshotStore` de `persistent-entity` (`persistence.rs:733`) ignora `tenant_id`, un defecto confirmado de aislamiento de tenants. No se corrige aquí (NG-1, D-9). Seguimiento F-5: una corrección revisada de forma independiente, con sus propios tests, independiente de la serie CORE-PERSIST.
- **KD-6 → F-6** — `InMemoryEventStore`/`StagingUnitOfWork` de `persistent-entity` (`persistence.rs:571`) es una bifurcación no consolidada que lleva `with_version_offset()`. No se fusiona aquí (NG-2, D-9). Seguimiento F-6: decidir fusionar-en-el-canónico vs. mantener-la-bifurcación-con-razón-declarada.
- **Límite del effect-store (D-9/D-10)** — `InMemoryEffectStore` y sus tres puertos (`EffectStateStore`, `EffectDedupStore`, `RetentionMaintenance`) permanecen en `ego-runtime`, sin tocar (NG-6, R12, R18). Seguimiento **CORE-PERSIST-E** (F-1): reubicar primero los puertos, luego consolidar la implementación.
- También nombrado, pero no cubierto por este cambio: sin consolidación de Postgres (NG-3 → CORE-PERSIST-C), sin expansión del harness de conformidad (NG-4, KD-4 → CORE-PERSIST-D, F-4).

## Auditoría de Trazabilidad

| Requisito | Tarea(s) que lo cubren |
|---|---|
| R1 — Propiedad canónica | 3.1–3.4, 4.1, 7.3, 8.1, 11.2–11.3 |
| R2 — Sin declaración duplicada | 5.6, 9.7, 12.5, 13.3 |
| R3 — Fakes nombrados no promovidos | 12.3, 12.4 |
| R4 — Lo faltante sigue faltando | 13.4 |
| R5 — Preservación de comportamiento | 3.1–3.4, 5.4, 5.6, 7.3, 9.5, 9.7, 11.2–11.3, 12.5 |
| R6 — Preservación de durabilidad/producción | 13.5 |
| R7 — Neutralidad de backend | 1.1, 13.6 |
| R8 — Consolidación de read-side | 10.1, 11.2–11.3, 11.4 |
| R9 — Re-exports de compatibilidad | 2.1, 4.1, 5.2, 5.5, 6.1, 8.1, 9.2, 9.6 |
| R10 — Propiedad única por puerto | 13.3 |
| R11 — Integridad de dependencias | 1.1, 1.4, 5.3, 7.1, 9.4, 13.1, 13.6 |
| R12 — Integridad del alcance de efectos | 13.2 |
| R13 — Sin refactor de Postgres | 13.2 |
| R14 — Sin expansión de conformidad | Diferido / Fuera de Alcance |
| R15 — Sin rediseño de contrato | 13.2 |
| R16 — Ningún test double promovido | 8.1, 9.5 |
| R17 — Duplicados de persistent-entity nombrados | Diferido / Fuera de Alcance |
| R18 — Límite del effect-store nombrado | Diferido / Fuera de Alcance |

**Verificación cruzada del límite de alcance contra NG-1..NG-12 de la propuesta — sin hallazgos.**
Ninguna tarea toca `crates/persistent-entity/`, `crates/runtime/`, `crates/effect-store/`, ni
`crates/persistence/`; ninguna tarea agrega una implementación de `ProjectionStateStore`, un
harness de conformidad, ni un archivo de Postgres; ninguna tarea promueve `TestClock` ni ningún
`Fake*Durable*`/double local `#[cfg(test)]`.
