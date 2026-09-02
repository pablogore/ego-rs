# Tareas: CORE-PERSIST-A — Superficie Unificada de la API de Persistencia (Puertos Propiedad del Dominio)

> Compañero de revisión en español. Fuente de verdad canónica: `tasks.md` (identificadores 1:1).
> TDD estricto: el archivo `reexport_identity.rs` de cada PR crece en ROJO (los nuevos
> testigos de identidad no resuelven) antes de su propia reubicación en VERDE, según la
> Estrategia de Pruebas de design.md. Los módulos `#[cfg(test)]` reubicados se mueven
> textualmente junto con su archivo — el conteo de aserciones antes/después debe coincidir
> exactamente (D-6, SC-3). El orden de las porciones es el orden obligatorio de AD-6 de
> design.md: `read_side/` → `operation/` → `persistence/` (EC-3), cada una compilando de
> forma independiente en todo el workspace antes de que empiece la siguiente.

## Pronóstico de Carga de Revisión

**Solo una estimación — no confirmada por una conversación con el propietario del cambio.**
Basada en la estimación de 1.500–2.000 líneas totales reubicadas del explore §11 (los
movimientos textuales cuentan como adición+eliminación completa aunque no cambie lógica) y en
la división de tres porciones de AD-6 de design.md.

| Campo | Valor |
|-------|-------|
| Líneas cambiadas estimadas | ~1.600–2.000 en total — PR1 ~350–500 (7 archivos hoja pequeños + esqueleto + prueba de `layers.rs`), PR2 ~700–950 (la más grande: `reservation.rs` es el archivo individual más grande, más la reubicación del macro `id_type!`), PR3 ~550–750 (`event_store.rs`, `repository.rs`, `event.rs`, más las verificaciones finales de diff de todo el cambio) |
| Riesgo del presupuesto de 400 líneas | Alto para las tres porciones — la reubicación textual mueve el texto completo, incluyendo comentarios de documentación y pruebas existentes, no un diff resumido |
| PRs encadenados recomendados | Sí |
| División sugerida | PR 1 (S1 — lado de lectura) → PR 2 (S2 — operación) → PR 3 (S3 — persistencia) |
| Estrategia de entrega | ask-on-risk (valor por defecto de la sesión) |
| Estrategia de cadena | **confirmada** — cadena stacked estricta, fusionada en orden PR1 → PR2 → PR3, cada una construida sobre la anterior; PR2 se rama de PR1, PR3 se rama de PR2, según "cada porción compila en todo el workspace antes de que empiece la siguiente" de AD-6 (decisión del propietario del cambio, 2026-09-02) |

Decisión necesaria antes de aplicar: No — resuelta por el propietario del cambio (2026-09-02)
PRs encadenados recomendados: Sí
Estrategia de cadena: confirmada — cadena stacked estricta, PR1 → PR2 → PR3, fusionada en orden, nunca tres PRs independientes contra `develop`
Riesgo del presupuesto de 400 líneas: Alto — excepción de PR2 preaprobada condicionalmente (ver nota abajo)

**Nota de presupuesto de revisión:** se espera que cada porción exceda las 400 líneas porque
la reubicación es textual (D-6) — ninguna línea se reescribe para reducir el diff. Dividir
más allá de S1/S2/S3 violaría las fronteras de cierre de ítems de AD-6 (EC-3):
`persistence/` necesita `OperationKey`/`OperationReceipt` de S2, así que no puede moverse
antes que `operation/`. Si PR2 excede el presupuesto con más severidad, es una desviación
aceptada de la misma forma que el PR2 de PROD-014B — nunca separar la definición de un ítem de
sus propias pruebas reubicadas solo para forzarlo bajo el presupuesto.

**Excepción de presupuesto para PR2 (decisión del propietario del cambio, 2026-09-02):**
preaprobada condicionalmente — el exceso sobre 400 líneas debe venir exclusivamente de la
reubicación mecánica de archivos de `operation/` más el movimiento del macro `id_type!` (Fase
7), sin comportamiento mezclado. Si una lectura de diff encuentra en PR2 cualquier cambio que
no sea move/import/re-export, la excepción queda anulada: detenerse y volver a dividir en
lugar de fusionar. Separar artificialmente el macro del resto de `operation/` para entrar en
el presupuesto queda rechazado — fragmentaría las pruebas reubicadas de un mismo ítem (mismo
principio que la nota de presupuesto de revisión anterior).

### Compuerta de Diff Semántico Cero (decisión del propietario del cambio, 2026-09-02)

Cada PR debe demostrar **diff semántico cero** antes de fusionar — que `cargo build`/`cargo
test` pasen es necesario pero no suficiente. Por cada PR, antes de fusionar:

- Comparar las firmas públicas (`pub fn`/`pub struct`/`pub trait`/`pub enum`/`pub const`/…) de
  cada ítem reubicado en su ruta anterior vs. su ruta nueva — deben ser idénticas salvo por la
  ruta de módulo en sí.
- Comparar la superficie de reexportación visible externamente de `ego_domain` (cada ruta
  todavía alcanzable desde `ego_domain::*` después del PR) antes vs. después — rutas
  idénticas, visibilidad idéntica.
- Comparar el conteo de aserciones de cada módulo `#[cfg(test)]` reubicado antes vs. después —
  idéntico (ya exigido por SC-3/SC-5; se reafirma aquí como condición explícita de la
  compuerta).
- **Cualquier cambio que no sea move/import/re-export detiene la ejecución de inmediato** —
  frenar la tarea, no continuar a la siguiente tarea, fase o PR, y elevarlo al propietario del
  cambio antes de seguir.

### Unidades de Trabajo Sugeridas

| Unidad | Objetivo | PR | Se rama de | Comando de prueba enfocado | Arnés de runtime | Frontera de rollback |
|------|------|-----|----------------|----------------------|-----------------|-------------------|
| 1 | S1 — lado de lectura: esqueleto de crate, `layers.toml`, relajación de compuerta de AD-1 + su prueba, `read_side/{offset,dedup,store,projection_state,event_tag,state,event_stream}`, reexportaciones de módulo | PR 1 | `develop` | `cargo build -p ego-persistence-api && cargo test -p xtask` | N/A — reubicación estructural, ningún comportamiento de runtime que probar (OOS-6) | Borrar `crates/persistence-api/`, restaurar los 7 módulos de `ego-domain`, quitar las ediciones de `Cargo.toml`/`layers.toml` y la relajación de `layers.rs` |
| 2 | S2 — operación: `operation/{key,receipt,reservation}`, reubicación del macro `id_type!` + `TenantId`, reexportaciones de módulo | PR 2 | PR 1 | `cargo build -p ego-persistence-api && cargo test --workspace` | N/A — reubicación estructural, ningún comportamiento de runtime que probar (OOS-6) | Borrar los 3 archivos `operation/` reubicados + el macro, restaurar los originales de `ego-domain`; PR 1 permanece válido y sin uso fuera de esta porción |
| 3 | S3 — persistencia: `persistence/{error,event_store,repository,snapshot,stored_event,tenant}`, `event.rs`/`DomainEvent`, reexportaciones de módulo, verificación final de diff de todo el cambio | PR 3 | PR 2 | `cargo build -p ego-persistence-api && cargo test --workspace` | N/A — reubicación estructural, ningún comportamiento de runtime que probar (OOS-6) | Borrar los 7 archivos restantes reubicados, restaurar los originales de `ego-domain`; PR 1–2 permanecen válidos para cualquier otro consumidor |

## Fase 1: Esqueleto de Crate y Compuerta de Capas (Fundamento) — PR 1

- [ ] 1.1 ROJO: agregar una prueba `#[cfg(test)]` en `xtask/src/layers.rs` que afirme que `domain → domain` pasa `check_direction` y que `domain → foundation`/`domain → infrastructure`/`domain → sdk` siguen fallando, siguiendo la forma de prueba existente `graph_from`/`layers_from` (`layers.rs:164-208`). Falla al compilar/pasar contra el `Some(&[])` actual (AD-1, SC-7).
- [ ] 1.2 VERDE: `xtask/src/layers.rs:76` — cambiar `"domain" => Some(&[])` a `"domain" => Some(&["domain"])`. Pone en verde 1.1.
- [ ] 1.3 Crear el esqueleto de `crates/persistence-api/`: `Cargo.toml` (paquete `ego-persistence-api`, dependencias derivadas del bloque de `ego-domain` según AD-5 — sin dependencia `path` de workspace), `src/lib.rs`. Agregarlo como miembro del workspace en el `Cargo.toml` raíz.
- [ ] 1.4 Agregar entrada en `layers.toml`: `"ego-persistence-api" = "domain"` (IS-5, FR-001).
- [ ] 1.5 Agregar una arista de dependencia `path` en `crates/domain/Cargo.toml` hacia `ego-persistence-api` (D-2, la única arista nueva del grafo de crates de este cambio).

## Fase 2: ROJO — Prueba de Identidad de Reexportación, Ítems de S1 — PR 1

- [ ] 2.1 Crear `crates/persistence-api/tests/reexport_identity.rs` con un testigo de identidad por cada ítem de S1 (`OffsetStore`, `Offset`, `OffsetStoreError`, `DedupStore`, `DedupStoreError`, `ReadSideStore`, `ReadSideStoreError`, `ProjectionStateStore`, `ProjectionStateStoreError`, `EventTag`, `ProjectionState`, `EventStreamElement`) — los traits object-safe reciben una coerción de identidad, los ítems genéricos un testigo con cláusula `where` que porta ambos bounds. Falla al compilar: ninguno de estos caminos existe todavía en `ego_persistence_api::*` (IS-6, SC-1).

## Fase 3: VERDE — Reubicar Archivos de `read_side/` — PR 1

- [ ] 3.1 Mover `crates/domain/src/read_side/offset.rs` textualmente (comentarios de documentación, `#[cfg(test)]`, la impl de reenvío `Arc<T>` en la línea 92) a `crates/persistence-api/src/read_side/offset.rs` (D-6, SC-4).
- [ ] 3.2 Mover `read_side/dedup.rs` textualmente (incluyendo la impl de reenvío `Arc<T>` en la línea 60) a `crates/persistence-api/src/read_side/dedup.rs` (SC-4).
- [ ] 3.3 Mover `read_side/store.rs` textualmente a `crates/persistence-api/src/read_side/store.rs`.
- [ ] 3.4 Mover `read_side/projection_state_store.rs` textualmente a `crates/persistence-api/src/read_side/projection_state.rs` — cero implementaciones, cero consumidores, sin cambios (D-8, AD-7).
- [ ] 3.5 Mover `read_side/event_tag.rs`, `read_side/state.rs`, `read_side/event_stream.rs` textualmente a las mismas rutas bajo `crates/persistence-api/src/read_side/` (AD-2, EC-1).

## Fase 4: VERDE — Reexportaciones de Módulo y Verificación de Consumidores, S1 — PR 1

- [ ] 4.1 Reemplazar cada módulo vaciado `crates/domain/src/read_side/{offset,dedup,store,projection_state_store,event_tag,state,event_stream}.rs` por una reexportación de módulo (`pub use ego_persistence_api::read_side::{...};`), dejando intactas y textuales las líneas `pub use` a nivel de ítem existentes (AD-4, D-5).
- [ ] 4.2 Confirmar que `read_side/scheduler.rs:5-10`, `session.rs:5-13`, `runner.rs:3-10` compilan sin editar — la reexportación de módulo resuelve sus imports `super::`/`crate::` sin ningún cambio (IS-4 se colapsa a cero ediciones, según AD-4).

## Fase 5: Verificación — PR 1

- [ ] 5.1 `cargo build -p ego-persistence-api` compila de forma independiente (FR-005, AD-5).
- [ ] 5.2 `cargo build --workspace` compila; pone en verde los testigos de identidad de 2.1 (IS-6, SC-1).
- [ ] 5.3 `cargo run -p xtask -- verify-layers` pasa: `ego-persistence-api` está mapeado, la arista está permitida, no hay ciclo (SC-6).
- [ ] 5.4 `cargo test --workspace` pasa, cero fallas nuevas, cero cambios en el conteo de aserciones de los módulos `#[cfg(test)]` movidos (SC-3, SC-5).
- [ ] 5.5 Lectura de diff: ninguna edición de `use` o `Cargo.toml` fuera de `crates/domain/` y `crates/persistence-api/` (SC-2).
- [ ] 5.6 Compuerta de diff semántico cero: comparar firmas públicas y la superficie de reexportación de `ego_domain` para cada ítem de S1, ruta anterior vs. ruta nueva — idénticas salvo por la ruta de módulo. Cualquier cambio que no sea move/import/re-export detiene el PR (compuerta del propietario del cambio, 2026-09-02).

## Fase 6: ROJO — Extensión de la Prueba de Identidad de Reexportación, Ítems de S2 — PR 2

- [ ] 6.1 Extender `reexport_identity.rs` con testigos para `OperationKey`, `OperationKeyError`, `OperationFingerprint`, `OperationKeyHash`, `MAX_LEN`, `OperationReceipt`, `AggregateOutcome`, `AggregateOutcomeError`, `OperationReservationStore`, `ReservationError`, `ReserveRequest`, `ReservationOutcome`, `Lease`, `OwnerFence`, `FencingToken`, `OldestCompleted`, `OperationId`, `OwnerId`, `StoredServiceResponse`, `TenantId`, `TenantIdError`. Falla al compilar hasta que las Fases 7/8 aterricen (IS-6, SC-1).

## Fase 7: VERDE — Reubicar Archivos de `operation/` y el Macro `id_type!` — PR 2

- [ ] 7.1 Mover `crates/domain/src/operation/key.rs` textualmente a `crates/persistence-api/src/operation/key.rs` (incluye `MAX_LEN`, `OperationFingerprint`, `OperationKeyHash`, D-7).
- [ ] 7.2 Mover `operation/receipt.rs` textualmente a `crates/persistence-api/src/operation/receipt.rs`.
- [ ] 7.3 Mover `operation/reservation.rs` textualmente a `crates/persistence-api/src/operation/reservation.rs`.
- [ ] 7.4 Mover el bloque `macro_rules! id_type` (`context.rs:7-54`) textualmente a `ego-persistence-api`, agregar `#[macro_export]`, e invocarlo ahí para generar `TenantId`/`TenantIdError` (AD-3, EC-2). Una sola definición del generador, no dos.

## Fase 8: VERDE — Reexportaciones de Módulo y Reinvocación del Macro, S2 — PR 2

- [ ] 8.1 Reemplazar `crates/domain/src/operation/{key,receipt,reservation}.rs` con reexportaciones de módulo de `ego_persistence_api::operation::{key,receipt,reservation}` (AD-4).
- [ ] 8.2 `crates/domain/src/context.rs`: eliminar la definición local de `id_type!`, reinvocar el macro reexportado para `AggregateId`, `EntityId`, `CorrelationId`, `CausationId`, `RequestId`; reexportar `TenantId`/`TenantIdError` en `ego_domain::context::TenantId` y `ego_domain::TenantId` (`lib.rs:103-107`) (AD-3).

## Fase 9: Verificación — PR 2

- [ ] 9.1 `cargo build -p ego-persistence-api` compila de forma independiente.
- [ ] 9.2 `cargo build --workspace` compila.
- [ ] 9.3 `cargo run -p xtask -- verify-layers` sigue pasando; `cargo test --workspace` cero fallas nuevas, cero cambios en el conteo de aserciones (SC-3, SC-5, SC-6).
- [ ] 9.4 Lectura de diff: sigue sin haber edición de `use`/`Cargo.toml` fuera de las dos crates; confirmar que existe exactamente una definición de `id_type!` en todo el workspace (SC-2, escenario de spec "solo existe una definición de `id_type!` en todo el workspace").
- [ ] 9.5 Compuerta de diff semántico cero: comparar firmas públicas y la superficie de reexportación de `ego_domain` para cada ítem de S2 + el macro `id_type!` reubicado, ruta anterior vs. ruta nueva — idénticas salvo por la ruta de módulo. Esta es también la comprobación que decide la excepción de presupuesto de PR2 de arriba: cualquier cambio que no sea move/import/re-export anula la excepción y detiene el PR (compuerta del propietario del cambio, 2026-09-02).

## Fase 10: ROJO — Extensión de la Prueba de Identidad de Reexportación, Ítems de S3 — PR 3

- [ ] 10.1 Extender `reexport_identity.rs` con testigos para `PersistenceError`, `EventStore`, `EventStoreUnitOfWork`, `Repository`, `Snapshot`, `StoredEvent`, `resolve_tenant`, `DomainEvent` — el archivo ahora cubre los 35 ítems (EC-4 de design). Falla al compilar hasta que las Fases 11/12 aterricen.

## Fase 11: VERDE — Reubicar Archivos de `persistence/` y `event.rs` — PR 3

- [ ] 11.1 Mover `crates/domain/src/persistence/error.rs` textualmente a `crates/persistence-api/src/persistence/error.rs`.
- [ ] 11.2 Mover `persistence/event_store.rs` textualmente (forma `async-trait` textual, OOS-4) a `crates/persistence-api/src/persistence/event_store.rs`.
- [ ] 11.3 Mover `persistence/repository.rs` textualmente a `crates/persistence-api/src/persistence/repository.rs`.
- [ ] 11.4 Mover `persistence/snapshot.rs` textualmente a `crates/persistence-api/src/persistence/snapshot.rs`.
- [ ] 11.5 Mover `persistence/stored_event.rs` textualmente a `crates/persistence-api/src/persistence/stored_event.rs`.
- [ ] 11.6 Mover `persistence/tenant.rs` textualmente (la regla de tres vías de `resolve_tenant` sin cambios, OOS-5) a `crates/persistence-api/src/persistence/tenant.rs`.
- [ ] 11.7 Mover `crates/domain/src/event.rs` textualmente (62 líneas, dependencias `chrono` + `serde_json`) a `crates/persistence-api/src/event.rs` (AD-2).

## Fase 12: VERDE — Reexportaciones de Módulo, S3 — PR 3

- [ ] 12.1 Reemplazar `crates/domain/src/persistence/{error,event_store,repository,snapshot,stored_event,tenant}.rs` y `crates/domain/src/event.rs` con reexportaciones de módulo de `ego_persistence_api::{persistence::{...}, event}`, dejando intactas y textuales las líneas `pub use` a nivel de ítem existentes (AD-4).

## Fase 13: Verificación y Chequeos de Diff de Todo el Cambio — PR 3

- [ ] 13.1 `cargo build -p ego-persistence-api` compila de forma independiente; `cargo build --workspace` compila.
- [ ] 13.2 `cargo run -p xtask -- verify-layers` pasa; `cargo test --workspace` cero fallas nuevas, cero cambios en el conteo de aserciones en cada módulo `#[cfg(test)]` reubicado (SC-3, SC-5, SC-6).
- [ ] 13.3 Lectura de diff sobre todo el cambio de tres PRs: cero archivo SQL/migración/esquema en el diff (SC-8, OOS-3); `crates/runtime/`, `crates/effect-store/`, y cada struct de implementación nombrado en OOS-1 están ausentes de toda lista de archivos (SC-9); ninguna crate fuera de `ego-domain`/`ego-persistence-api` tiene un `use` editado o una dependencia `Cargo.toml` agregada en los tres PRs combinados (SC-2).
- [ ] 13.4 Confirmar que KD-1 (`ProjectionStateStore`), KD-2 (el defecto `ON CONFLICT`/tenant de `PostgreSQLRepository`), KD-3 (`persistent-entity/src/types.rs`), KD-4 (asimetría de conformidad) permanecen sin modificar — arrastrados, no corregidos ni eliminados (SC-11).
- [ ] 13.5 Compuerta de diff semántico cero: comparar firmas públicas y la superficie de reexportación de `ego_domain` para cada ítem de S3 + `event.rs`, ruta anterior vs. ruta nueva — idénticas salvo por la ruta de módulo. Cualquier cambio que no sea move/import/re-export detiene el PR — esta es la compuerta final antes de que todo el cambio de tres PRs esté completo para aplicar (compuerta del propietario del cambio, 2026-09-02).

## Fase 14: Trazabilidad Post-Merge — Futuro `sdd-archive`

- [ ] 14.1 No se realiza en esta sesión. Una vez que los tres PRs se fusionen, `sdd-archive` fusiona la capability `persistence-api-surface` (NUEVA) de `openspec/changes/core-persist-a-unified-persistence-api-surface/spec.md` en `openspec/specs/persistence-api-surface/spec.md`, y fusiona el bloque `FR-002` del delta `foundation-integrity` (MODIFICADA) en `openspec/specs/foundation-integrity/spec.md` — esa fusión es trabajo de `sdd-archive`, no de esta fase ni de `sdd-apply`.

## Auditoría de Trazabilidad

Todos los requisitos de spec mapeados a al menos una tarea que los cubre:

| Requisito | Capability | Tarea(s) que lo cubren |
|---|---|---|
| Cada Ítem Reubicado Se Mueve Textualmente | `persistence-api-surface` | 3.1–3.5, 7.1–7.4, 11.1–11.7 |
| La Ruta Antigua Resuelve al Mismo Ítem | `persistence-api-surface` | 4.1, 8.1, 12.1, 2.1, 6.1, 10.1 |
| La Forma del Trait Es Idéntica Byte a Byte | `persistence-api-surface` | 3.1–3.3, 7.2–7.3, 11.2, 13.2 |
| Las Implementaciones de Reenvío `Arc<T>` Se Mueven Intactas | `persistence-api-surface` | 3.1, 3.2, 5.4 |
| El Macro `id_type!` Se Reubica y Se Reinvoca, No Se Duplica | `persistence-api-surface` | 7.4, 8.2, 9.4 |
| Ningún Consumidor Fuera de las Dos Crates Es Editado | `persistence-api-surface` | 4.2, 5.5, 9.4, 13.3 |
| `ego-persistence-api` No Depende de Ninguna Crate del Workspace | `persistence-api-surface` | 1.3, 5.1, 9.1, 13.1 |
| Los Ítems Conocidos Como Muertos Se Reubican Sin Comportamiento Nuevo | `persistence-api-surface` | 3.4, 13.4 |
| FR-002 — Aplicación de la Dirección de Dependencias (auto-arista de dominio) | `foundation-integrity` (MODIFICADA) | 1.1, 1.2 |

**Verificación cruzada de frontera de alcance contra los OOS-1..14 de la proposal — cero
hallazgos.** Ninguna tarea de esta lista toca un struct de implementación (OOS-1),
`ego-runtime`/`ego-effect-store` (OOS-2), SQL o migraciones (OOS-3), una firma de trait
(OOS-4), semántica de tenant (OOS-5), una fusión de crates (OOS-7), una capability nueva
(OOS-8), la eliminación de `ProjectionStateStore` (OOS-9), un arnés de conformidad (OOS-14), ni
el defecto `42P10` de `PostgreSQLRepository` (OOS-12, KD-2, F-2 — un seguimiento independiente,
no supeditado a esta serie).
