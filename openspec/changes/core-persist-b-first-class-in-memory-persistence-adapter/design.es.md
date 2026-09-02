# Diseño: CORE-PERSIST-B — Adaptador de persistencia en memoria de primera clase

> Compañero de revisión en español. Documento canónico / fuente de verdad: `design.md`
> (identificadores 1:1: EC-1..EC-7, AD-1..AD-10, S1..S3, OQ-1..OQ-3).
>
> **Entradas**: `proposal.md` (D-1..D-12, NG-1..NG-12, IS-1..IS-6, R1..R18, R-1..R-7, KD-1/4/5/6,
> F-1/F-4/F-5/F-6) y `explore.md` (MOVE MATRIX, COMPATIBILITY REEXPORT MATRIX, DEPENDENCY GRAPH,
> EFFECT STORE BLOCKER ANALYSIS, TARGET CRATE / MODULE TREE). Este documento decide **el cómo**:
> el cierre de dependencias del crate, su árbol de módulos, la reescritura exacta de imports, la
> granularidad de la re-exportación en cada crate que cede código, dónde vive la prueba de
> compatibilidad y los límites de cada rebanada. Los requisitos observables pertenecen a
> `spec.md` y no se repiten aquí.
>
> **Línea base leída**: `develop` @ `e74c9fc`. Cada `archivo:línea` de abajo fue leído sobre esta
> línea base, no recordado de las entradas. Donde la línea base contradice una entrada, queda
> registrado como **Corrección de Evidencia**, no aplicado en silencio.

## Enfoque técnico

Un crate nuevo, dos aristas de dependencia unidireccionales, tres crates que ceden código, y una
capa de compatibilidad cuya granularidad se elige *por crate cedente* según cuál sea realmente la
ruta pública de ese crate — no según una regla uniforme.

`ego-persistence-memory` depende de `ego-persistence-api` para cada puerto que implementa, y de
`ego-domain` para exactamente un elemento (`Clock`, `crates/domain/src/time/clock.rs:24`). Ambos
destinos son crates de capa `domain` y el crate nuevo es `foundation`, de modo que ambas aristas
ya están permitidas por `xtask/src/layers.rs:77` y **no se requiere ninguna edición de la matriz
de la compuerta** (AD-1). Los tres crates cedentes ganan una dependencia normal hacia el crate
nuevo y conservan cada ruta que un consumidor resuelve hoy.

**No se incluye diagrama de secuencia, y es una decisión deliberada de aplicabilidad.** La regla
de diseño de `openspec/config.yaml` pide uno para flujos asíncronos complejos. Este cambio agrega,
elimina y reordena cero rutas de llamada: cada cuerpo de método `#[async_trait]` se mueve
byte-idéntico (D-4, R5), así que un diagrama dibujado aquí representaría un flujo que el cambio no
toca. La estructura portante es el **grafo de dependencias**, dado más abajo — la misma decisión
que tomó CORE-PERSIST-A por la misma razón (`design.md` archivado, `:27-31`).

---

## Correcciones de Evidencia

Siete. Cada una se encontró leyendo la línea base en lugar de las entradas, y cada una cambia lo
que la implementación debe hacer.

### EC-1 — `ego_persistence_api::operation` no aplana lo que `ego_domain::operation` aplana

D-4 dice que la reescritura mapea `use ego_domain::persistence::…` a
`use ego_persistence_api::persistence::…`. Esa forma uno-a-uno se cumple para `persistence::` y
`read_side::`, y **falla para `operation::`**:

| Módulo | `ego-domain` | `ego-persistence-api` |
|---|---|---|
| `persistence` | re-exportado íntegro | `mod.rs:39-44` exporta `PersistenceError`, `EventStore`, `EventStoreUnitOfWork`, `Repository`, `Snapshot`, `StoredEvent`, `resolve_tenant` — conjunto idéntico |
| `read_side` | re-exportado íntegro | `mod.rs:7-13` exporta los submódulos; ambos crates se direccionan por submódulo (`read_side::offset::Offset`) — forma idéntica |
| `operation` | `crates/domain/src/operation/mod.rs:26` aplana **todo el vocabulario de reserva** a `ego_domain::operation::{Lease, OwnerId, ReserveRequest, …}` | `crates/persistence-api/src/operation/mod.rs:18-19` aplana **solo** `OperationKey` y `OperationReceipt` |

Consecuencia: el import de once nombres de `crates/testkit/src/reservation.rs:16-19` no puede
reescribirse cambiando el prefijo del crate. Su destino correcto es
`ego_persistence_api::operation::reservation::{…}` (los once elementos están declarados allí —
`reservation.rs:48,66,178,205,231,273,290,309,321,346,388`). `OperationReceipt`
(`event_store.rs:6`) *sí* se cambia uno-a-uno. **AD-4 fija los bloques exactos.**

### EC-2 — `reservation.rs:70` es una ruta calificada **en línea**, no una línea `use`

D-4 permite «reescribir líneas `use`» y nada más. La estructura que se mueve lleva una expresión
de ruta en su cuerpo:

```rust
// crates/testkit/src/reservation.rs:68-72
struct Record {
    fingerprint: ego_domain::operation::OperationFingerprint,
    state: RecordState,
}
```

`OperationFingerprint` se genera en `ego-persistence-api` (`operation/key.rs`) y llega a
`ego_domain::operation` mediante `crates/domain/src/operation/mod.rs:21,24`. Dejada tal cual sigue
compilando (el crate nuevo depende de `ego-domain` de todos modos) y es byte-idéntica; reescrita a
`ego_persistence_api::operation::key::OperationFingerprint` nombra **el mismo elemento** y mantiene
la superficie `ego_domain::` del crate en exactamente una línea. **AD-5 decide; OQ-1 pide a la
fase propose ampliar la redacción de D-4** de «líneas `use`» a «expresiones de ruta que nombran un
elemento de puerto reubicado».

### EC-3 — La re-exportación de compatibilidad del store de reservas debe vivir en `reservation.rs`, no en `lib.rs`

D-5 la ubica en `crates/testkit/src/lib.rs`. Esa ubicación es insuficiente. `reservation.rs`
contiene dos módulos `#[cfg(test)]` colocados que resuelven el store a través del **módulo**, no
de la raíz del crate:

- `crates/testkit/src/reservation.rs:378` — `use super::{InMemoryOperationReservationStore, TestClock};`
- `crates/testkit/src/reservation.rs:525` — la misma línea, en `mod oldest_completed_contract`

Una re-exportación solo en `lib.rs` deja ambas líneas `use super::` sin resolver, forzando una
edición a módulos de prueba que D-8 declara intactos. Un `pub use` **dentro de `reservation.rs`**
hace que `super::InMemoryOperationReservationStore` resuelva, y deja
`crates/testkit/src/lib.rs:50` (`pub use reservation::{InMemoryOperationReservationStore, TestClock};`)
byte-idéntico. **AD-5.**

### EC-4 — Solo uno de los cuatro archivos de `ego-infrastructure` lleva módulo de pruebas

`#[cfg(test)]` aparece exactamente una vez bajo
`crates/infrastructure/src/persistence/in_memory/`: `read_side_store.rs:142`, cuyo módulo usa
`chrono::Utc` (`:144`), `serde_json::json!` (`:158`) y `#[tokio::test]` (`:165`).
`event_store.rs`, `repository.rs` y `snapshot.rs` **no tienen ninguno** — su cobertura vive en
`crates/infrastructure/tests/` (`in_memory_event_store_conformance.rs:17-18`,
`commit_publishes_atomically.rs:25`), que se queda y sigue compilando a través de la
re-exportación. Esto fija las `[dev-dependencies]` del crate nuevo en `tokio` y nada más. **AD-2.**

### EC-5 — El movimiento de reference-app toca **dos** archivos, no uno

D-6 / IS-4 nombran `examples/reference-app/src/read_side/store.rs`. Dos hechos más en la línea
base:

1. `examples/reference-app/src/read_side/mod.rs:36-39` re-exporta públicamente ambos tipos que se
   mueven (`pub use store::{FakeDurableDedupStore, FakeDurableOffsetStore, InMemoryDedupStore, InMemoryOffsetStore, ReadSideSink, SharedReadSideStore};`)
   y `:106-107` los construye en `ReadSideHandles::in_memory()`.
2. `store.rs:251` y `:282` — `FakeDurableOffsetStore(InMemoryOffsetStore)` y
   `FakeDurableDedupStore(InMemoryDedupStore)` **envuelven por valor a los tipos que se mueven** y
   delegan en ellos (`:265,275,296,305`). Se quedan (NG-8, R3), así que `store.rs` debe seguir
   nombrando ambos tipos después de que las declaraciones se vayan.

**AD-7** resuelve ambos sin cambiar la superficie pública propia del ejemplo.

### EC-6 — `crates/persistent-entity/src/builder.rs` queda confirmado como no afectado

La COMPATIBILITY REEXPORT MATRIX de `explore.md` dejó esto abierto («need to confirm at propose
time»). Confirmado sobre la línea base: `builder.rs:10` dice
`use crate::persistence::{InMemoryEventStore, InMemorySnapshotStore, PersistenceFacade};` — los
duplicados **propios** del crate (`persistence.rs:571,733`), que D-9 no mueve. `builder.rs:356,360`
construyen esos. No se requiere re-exportación alguna para `persistent-entity`, y sus dos pruebas
`try_build_rejects_explicit_in_memory_*` (`builder.rs:768,793`) ejercitan tipos que este cambio
nunca toca, razón por la cual R6 se cumple trivialmente.

### EC-7 — `chrono` es solo dev hasta que llega el store de reservas

Ninguna de las cuatro implementaciones de `ego-infrastructure` nombra `chrono` fuera del módulo de
pruebas de `read_side_store.rs` (EC-4). El store de reservas sí lo hace, en su cuerpo
(`reservation.rs:59,64,195,301` — `DateTime<Utc>`). Por lo tanto el conjunto de dependencias del
crate nuevo no es constante a través de las rebanadas: S1 necesita `chrono` solo como
dev-dependency, y S2 la promueve a dependencia normal. Se declara para que el límite de rebanada
sea derivable y no adivinado. **AD-2, AD-9.**

---

## Grafo de dependencias

**Antes** — las siete implementaciones viven en tres crates de tres capas distintas:

```
        ego-persistence-api  [domain]  ← hoja, dueña de los ocho puertos
                  ▲
              ego-domain     [domain]  ← los re-exporta en sus rutas antiguas
                  ▲
     ┌────────────┼──────────────┬───────────────────┐
ego-infrastructure    ego-testkit        reference-app
 [infrastructure]      [tooling, SUMIDERO] [no verificado por capas]
  4 implementaciones     1 implementación   2 implementaciones
```

**Después** — un crate nuevo bajo los tres consumidores, dos aristas salientes nuevas, tres
entrantes:

```
        ego-persistence-api  [domain]  ← sigue siendo hoja, sigue intacto (NG-5, R15)
                  ▲                ▲
                  │                │  puertos
              ego-domain  ────┐    │
               [domain]       │    │
                  ▲       Clock│    │
                  │            ▼    │
     ┌────────────┤     ego-persistence-memory  [foundation]
     │            │        (7 implementaciones)
     │            │              ▲   ▲   ▲
     │            │              │   │   │  re-exportación / import
ego-infrastructure   ego-testkit    reference-app
 [infrastructure]     [tooling]      [no verificado por capas]
```

**No se introduce ningún ciclo, y esto es un hecho sobre archivos, no una promesa de revisión.**
`ego-persistence-api` no nombra ninguna dependencia `path` del workspace
(`crates/persistence-api/Cargo.toml:6-20`), `ego-domain` depende únicamente de ella, y
`ego-persistence-memory` depende solo de esas dos. Las aristas inversas no pueden existir: Cargo
rechaza el ciclo antes de que `xtask verify-layers` se ejecute, y la verificación de ciclos de
FR-003 lo rechaza otra vez. Esto satisface por construcción la regla «No circular dependencies
between crates» de `openspec/config.yaml`.

---

## Decisiones de arquitectura

### AD-1 — La capa es `foundation`; el cambio es **una línea en `layers.toml`** y cero líneas en `xtask/`

**Decisión** — agregar a la tabla `[layers]` de `layers.toml` (hoy `:15-35`):

```toml
"ego-persistence-memory" = "foundation"
```

y a la lista `members` del workspace (`Cargo.toml:2-23`): `"crates/persistence-memory",`.
**`xtask/src/layers.rs` no se abre.**

**Criterios**:

1. **El mapeo crate→capa vive en `layers.toml`, no en `layers.rs`.** Verificado directamente:
   `layers.rs` contiene solo `KNOWN_LAYERS` (`:14-23`), la matriz `allowed_layers` (`:74-92`), las
   dos verificaciones (`:97-145`) y `load_layers_toml` (`:148-158`), que parsea la tabla
   `[layers]`. No hay ningún nombre de crate en `layers.rs` fuera de sus fixtures `#[cfg(test)]`.
   El «dondequiera que viva realmente el mapeo crate→capa» de D-2 resuelve a `layers.toml`.
2. **Cada arista que este cambio crea ya está permitida.** Ningún brazo de la matriz cambia:

   | Arista | Capas | Permitida por |
   |---|---|---|
   | `ego-persistence-memory → ego-persistence-api` | foundation → domain | `layers.rs:77` |
   | `ego-persistence-memory → ego-domain` | foundation → domain | `layers.rs:77` |
   | `ego-infrastructure → ego-persistence-memory` | infrastructure → foundation | `layers.rs:80-86` |
   | `ego-testkit → ego-persistence-memory` | tooling → * | `layers.rs:89` (`None` = sumidero) |
   | `reference-app → ego-persistence-memory` | — | fuera de alcance por completo (ver 4) |

3. **La entrada en `layers.toml` es obligatoria, no opcional.**
   `xtask/src/metadata.rs:82-85,100-105` restringe cada verificación a miembros del workspace cuyo
   manifiesto viva bajo `<raíz>/crates/`. `crates/persistence-memory/` califica, así que la
   verificación de completitud de FR-001 (`layers.rs:125-131`) emitiría `UnmappedCrate` sin la
   entrada. Esto satisface FR-001; no lo modifica (Capabilities de la propuesta →
   `foundation-integrity`: «no modification expected» — **confirmado**).
4. **La arista de reference-app no se verifica en absoluto.** `metadata.rs:100-105` y su prueba
   (`:209-220`) excluyen `examples/reference-app` y `xtask` de las tres verificaciones, así que la
   nueva dependencia de IS-4 no crea obligación alguna ante la compuerta en ninguna dirección.
5. **`domain` fue considerada y rechazada**, conforme a D-2(b): mapear el crate nuevo a `domain`
   cabalgaría sobre la auto-arista `"domain" => Some(&["domain"])` de CORE-PERSIST-A
   (`layers.rs:76`) y la ensancharía de «un crate de dominio puede alcanzar un crate de *puertos*»
   a «…puede alcanzar un *adaptador*», legalizando `ego-domain → ego-persistence-memory`.
   `foundation` cierra esa puerta: el conjunto permitido de `domain` no contiene `foundation`, y
   `layers.rs:259-275` ya lo afirma.

**Obsolescencia conocida, no corregida aquí**: el comentario de cabecera `layers.toml:6` sigue
diciendo `domain → nothing`, algo que `layers.rs:76` contradice desde CORE-PERSIST-A. La matriz
ejecutable es la autoridad. Intacto conforme a NG-7 y al riesgo R-6 de la propuesta.

### AD-2 — El conjunto de dependencias se deriva de los siete archivos, y `ego-domain` se gana exactamente una línea

**Decisión** — `crates/persistence-memory/Cargo.toml`:

```toml
[package]
name = "ego-persistence-memory"
version = "0.1.0"
edition = "2021"

[dependencies]
ego-persistence-api = { path = "../persistence-api" }
# Solo `Clock`. CORE-PERSIST-A no lo reubicó — sigue declarado en
# crates/domain/src/time/clock.rs:24 — y el store de reservas sostiene un
# `Arc<dyn Clock>` (D-3, EC-1). Este es el ÚNICO elemento de ego-domain del crate.
ego-domain = { path = "../domain" }
async-trait = { workspace = true }
chrono = "0.4"
serde_json = "1"

[dev-dependencies]
# El módulo `#[cfg(test)]` reubicado de read_side/store.rs: `#[tokio::test]` (EC-4).
tokio = { version = "1", features = ["macros", "rt"] }
```

**Criterios**:

1. **Cada entrada es trazable a un import de la línea base**, y no hay nada más presente:

   | Dependencia | Requerida por | Evidencia |
   |---|---|---|
   | `ego-persistence-api` | las siete | cada puerto y cada tipo de valor que nombran |
   | `ego-domain` | solo `operation/reservation.rs` | `reservation.rs:20` (`use ego_domain::Clock`), `:80` (`clock: Arc<dyn Clock>`) |
   | `async-trait` | event_store, read-side store, offset, dedup, reservation | `event_store.rs:4`, `store.rs:11`, `reservation.rs:14` |
   | `chrono` | solo reservation | `reservation.rs:59,64,195,301` (`DateTime<Utc>`) — EC-7 |
   | `serde_json` | read-side store, snapshot | `read_side_store.rs:13`, `snapshot.rs:5` |
   | `tokio` (dev) | módulo de pruebas reubicado del read-side store | `read_side_store.rs:165` — EC-4 |

2. **`serde` está ausente a propósito.** Ningún cuerpo movido deriva `Serialize`/`Deserialize`;
   los derives de `EventStreamElement` viven en `ego-persistence-api`
   (`read_side/event_stream.rs:12`), que ya trae su propio `serde`
   (`persistence-api/Cargo.toml:7`).
3. **La lista de R11 se cumple exactamente.** Sin `ego-application`, `ego-runtime`,
   `ego-infrastructure`, `ego-persistence`, `ego-testkit`, transporte ni ejemplo — y sin `sqlx`,
   sin OpenTelemetry, sin `dashmap`, satisfaciendo la cláusula de neutralidad de backend de R7.
   `ego-infrastructure` carga hoy todo eso (`infrastructure/Cargo.toml:8-20`) mientras su
   submódulo `in_memory` no importa nada de ello; esa brecha es lo concreto que este crate cierra.
4. **La arista `ego-domain` es verificable, no afirmada.** `rg '^use ego_domain::|ego_domain::'
   crates/persistence-memory/src` debe devolver exactamente una línea: el
   `use ego_domain::Clock;` de `operation/reservation.rs`. La reescritura de EC-2 en AD-5 es lo
   que hace que ese conteo sea uno y no dos, y Testing lo fija como propiedad del diff.
5. **No existe una tercera arista.** Se pide confirmarlo en lugar de asumirlo: las listas de
   import de las otras seis implementaciones (`event_store.rs:1-8`, `repository.rs:1-4`,
   `snapshot.rs:1-5`, `read_side_store.rs:6-11`, `store.rs:7-19`) nombran solo `std`,
   `async_trait`, `serde_json` y elementos que `ego-domain` re-exporta desde
   `ego-persistence-api`. Cada uno de esos elementos resuelve directamente desde
   `ego-persistence-api` (verificado contra
   `persistence-api/src/{lib.rs:18-34, persistence/mod.rs:39-44, read_side/mod.rs:7-13}`), así que
   ninguna de las seis necesita `ego-domain` en absoluto. **El riesgo R-3 de la «tercera arista»
   queda cerrado aquí, en tiempo de diseño, no en tiempo de aplicación.**

### AD-3 — El árbol de módulos refleja el de `ego-persistence-api`; sin aplanado en la raíz; sin nueva compuerta de lint

**Decisión** — refinando el árbol propuesto en `explore.md` en dos puntos:

```
crates/persistence-memory/            (paquete: ego-persistence-memory)
├── Cargo.toml                        # AD-2
└── src/
    ├── lib.rs                        # declaraciones de módulo + doc del crate, sin re-exportaciones
    ├── persistence/
    │   ├── mod.rs
    │   ├── event_store.rs            # InMemoryEventStore + InMemoryEventStoreUnitOfWork  ← infra event_store.rs
    │   ├── repository.rs             # InMemoryRepository                                  ← infra repository.rs
    │   └── snapshot.rs               # InMemorySnapshotStore (correcto en tenant)          ← infra snapshot.rs
    ├── read_side/
    │   ├── mod.rs
    │   ├── store.rs                  # InMemoryReadSideStore + paginate + su módulo de test ← infra read_side_store.rs
    │   ├── offset.rs                 # InMemoryOffsetStore                                 ← reference-app store.rs:153
    │   └── dedup.rs                  # InMemoryDedupStore                                  ← reference-app store.rs:199
    └── operation/
        ├── mod.rs
        └── reservation.rs            # InMemoryOperationReservationStore + Record/RecordState ← testkit reservation.rs
```

**Refinamiento 1** — `read_side/read_side_store.rs` → `read_side/store.rs`. **Criterios**: cada
otro archivo del árbol ya refleja su módulo de puerto uno a uno
(`persistence::event_store` ⇄ `ego_persistence_api::persistence::event_store`;
`read_side::offset` ⇄ `read_side::offset`). Conservar `read_side_store.rs` convierte al read-side
store en la única fila que un lector tiene que consultar. La repetición que el nombre original
evitaba (`in_memory::store`) no ocurre aquí — el nombre del crate desambigua
(`ego_persistence_memory::read_side::store::InMemoryReadSideStore` implementa
`ego_persistence_api::read_side::store::ReadSideStore`). `explore.md` marcó su árbol como «working
name, not forced»; las rutas canónicas de su COMPATIBILITY REEXPORT MATRIX se desplazan en
consecuencia y se reexponen completas en **Puntos de integración**.

**Refinamiento 2** — `lib.rs` declara módulos y no re-exporta nada en la raíz del crate.
**Criterios**: `ego-persistence-api` sienta este precedente (`lib.rs:18-34` — cinco `pub mod`,
cero `pub use`), y un aplanado en la raíz sería superficie pública nueva, que NG-11 prohíbe. Los
consumidores alcanzan los elementos en su ruta de módulo; la capa de compatibilidad de los crates
cedentes absorbe la verbosidad para todos los que ya tenían una ruta más corta.

**Refinamiento 3 (una trampa, nombrada)** — **el crate no lleva `#![deny(missing_docs)]`**, aunque
`ego-testkit` (`lib.rs:1`), `ego-security-sdk`, `security-apikey` y `security-jwt` sí lo lleven.
Varios elementos movidos no tienen comentario de documentación (p. ej. `InMemoryRepository::new`,
`repository.rs:16`; `InMemorySnapshotStore::new`, `snapshot.rs:17`). Añadir el lint forzaría
comentarios sobre cuerpos movidos — una edición de cuerpo, prohibida por D-4 y detectable como
violación de R5. Los crates de origen (`ego-infrastructure`, `reference-app`) tampoco llevan
atributo de lint a nivel de crate, así que esto preserva exactamente la postura de lint bajo la
que cada cuerpo compila hoy.

### AD-4 — La reescritura de imports se enumera por archivo, no se describe con una regla

**Decisión** — el conjunto completo de ediciones permitidas dentro de un cuerpo movido. Cada línea
de abajo fue leída sobre la línea base; cada `después` nombra el mismo elemento que su `antes` por
construcción (CORE-PERSIST-A entregó esa identidad; spec `persistence-api-surface`: «Old Path
Resolves To The Same Item»).

| # | Archivo (ruta nueva) | Antes | Después |
|---|---|---|---|
| 1 | `persistence/event_store.rs` | `use ego_domain::event::DomainEvent;` (`:5`) | `use ego_persistence_api::event::DomainEvent;` |
| | | `use ego_domain::operation::OperationReceipt;` (`:6`) | `use ego_persistence_api::operation::OperationReceipt;` |
| | | `use ego_domain::persistence::resolve_tenant;` (`:7`) | `use ego_persistence_api::persistence::resolve_tenant;` |
| | | `use ego_domain::persistence::{EventStore, EventStoreUnitOfWork, PersistenceError, StoredEvent};` (`:8`) | `use ego_persistence_api::persistence::{EventStore, EventStoreUnitOfWork, PersistenceError, StoredEvent};` |
| 2 | `persistence/repository.rs` | `:3-4` (`resolve_tenant`; `PersistenceError, Repository`) | los mismos nombres bajo `ego_persistence_api::persistence` |
| 3 | `persistence/snapshot.rs` | `:3-4` (`resolve_tenant`; `PersistenceError, Snapshot`) | los mismos nombres bajo `ego_persistence_api::persistence`; `:5` `use serde_json::Value;` **sin cambio** |
| 4 | `read_side/store.rs` | `:8-11` (`event_stream::EventStreamElement`, `event_tag::EventTag`, `offset::Offset`, `store::{ReadSideStore, ReadSideStoreError}`) | las mismas cuatro rutas de submódulo bajo `ego_persistence_api::read_side` |
| 5 | `read_side/offset.rs` | de reference-app `store.rs:16` — `use ego_domain::read_side::offset::{Offset, OffsetStore, OffsetStoreError};` | `use ego_persistence_api::read_side::offset::{Offset, OffsetStore, OffsetStoreError};` más `event_tag::EventTag` (`store.rs:15`) |
| 6 | `read_side/dedup.rs` | de reference-app `store.rs:13` — `use ego_domain::read_side::dedup::{DedupStore, DedupStoreError};` | `use ego_persistence_api::read_side::dedup::{DedupStore, DedupStoreError};` más `event_tag::EventTag` |
| 7 | `operation/reservation.rs` | `use ego_domain::operation::{FencingToken, Lease, OldestCompleted, OperationId, OperationReservationStore, OwnerFence, OwnerId, ReservationError, ReservationOutcome, ReserveRequest, StoredServiceResponse};` (`:16-19`) | **`use ego_persistence_api::operation::reservation::{…los mismos once nombres…}`** — el submódulo, no `operation::` (EC-1) |
| | | `fingerprint: ego_domain::operation::OperationFingerprint` (`:70`, en línea) | `ego_persistence_api::operation::key::OperationFingerprint` (EC-2, AD-5) |
| | | `use ego_domain::Clock;` (`:20`) | **sin cambio** — la única línea `ego_domain::` sobreviviente (AD-2, criterio 4) |

Dividir un archivo fuente en dos archivos destino (los ítems 5 y 6 provienen ambos de
`examples/reference-app/src/read_side/store.rs`) implica que cada destino lleva solo los imports
que su propio cuerpo nombra. Eso es una consecuencia mecánica de la división, no una edición de
cuerpo: la estructura, su `impl #[async_trait]`, sus alias de tupla clave (`DedupKey`/`OffsetKey`,
`store.rs:145,148`) y sus comentarios de documentación se mueven byte-idénticos, y
`std::sync::{Arc, Mutex}` junto con `std::collections::{HashMap, HashSet}` siguen al tipo que usa
cada uno.

**Nada más cambia en ningún cuerpo movido.** La lista de R5 — resolución de tenant, estrategia de
bloqueo, aritmética de conflicto de versión, la guarda fail-closed de tenant vacío en `paginate`
(`read_side_store.rs:111-115`) — queda intacta, y ningún tipo movido declara `is_durable()`, que
es lo que hace que R6 se cumpla sin una sola prueba nueva.

### AD-5 — `reservation.rs` se **divide**, no se mueve; su re-exportación de compatibilidad vive en el módulo

**Decisión** — `crates/testkit/src/reservation.rs` después del cambio contiene, en orden:

1. su documentación de módulo (`:1-9`), sin cambios;
2. un bloque de imports podado: `use std::sync::Mutex;`, `use chrono::{DateTime, Duration, Utc};`
   y `use ego_domain::Clock;` — todo lo que `TestClock` sigue necesitando, y nada más;
3. **`pub use ego_persistence_memory::operation::reservation::InMemoryOperationReservationStore;`**;
4. `TestClock` y su `impl Clock` (`:22-50`), byte-idénticos (D-8, R16);
5. ambos módulos `#[cfg(test)]` (`:370-512`, `:514-…`), **byte-idénticos**, incluidas sus líneas
   `use super::{InMemoryOperationReservationStore, TestClock};` en `:378` y `:525`.

`RecordState` (`:52-66`), `Record` (`:68-72`), el store (`:79-97`) y su
`impl OperationReservationStore` (`:99-…`) se reubican; `Record`/`RecordState` son privados y
viajan con el único tipo que los nombra.

**Criterios**:

1. **El `pub use` va en el módulo porque ahí es donde miran los dos módulos de prueba** (EC-3).
   Simultáneamente mantiene `crates/testkit/src/lib.rs:50` byte-idéntico, de modo que el cambio le
   cuesta a `lib.rs` cero ediciones. Es la misma intuición de AD-4 de CORE-PERSIST-A — poner la
   re-exportación donde las rutas existentes ya resuelven — aplicada a otra forma.
2. **Podar los imports del archivo cedente es inevitable y está dentro del alcance.**
   `async_trait`, los once nombres de `operation`, `HashMap` y `Arc` se van con el store; dejarlos
   sería una advertencia de import sin usar, y `make clippy` corre con `-D warnings`. Esta edición
   está dentro del crate cedente, que D-5 ya abre.
3. **Un `pub use` satisface `#![deny(missing_docs)]`** (`testkit/src/lib.rs:1`): una
   re-exportación hereda la documentación del destino, y el comentario del store
   (`reservation.rs:74-78`) se mueve con él.
4. **Las dos suites de conformidad no se ven afectadas.**
   `crates/testkit/src/reservation_conformance.rs` es genérica sobre
   `OperationReservationStore` y se exporta por separado (`lib.rs:51-54`); no nombra el store
   concreto ni cambia.

**La ruta en línea de EC-2 se reescribe** (segunda fila del ítem 7 en AD-4): `Record` es privada,
las dos rutas nombran un solo elemento, y reescribirla mantiene la superficie `ego_domain::` del
crate en exactamente una línea localizable con `grep`. **OQ-1** pide a la fase propose ampliar la
redacción de D-4 para que coincida; nada de la decisión cambia si se rechaza — la alternativa es
dejar `:70` byte-idéntica, lo cual también compila, y solo le cuesta al criterio 4 de AD-2 una
excepción.

### AD-6 — La re-exportación de `ego-infrastructure` es a granularidad de **elemento** — lo opuesto a AD-4 de CORE-PERSIST-A, por una razón hallada en el código

**Decisión** — `crates/infrastructure/src/persistence/in_memory/mod.rs` queda así, conservando su
documentación de módulo (`:1-5`) sin cambios:

```rust
pub use ego_persistence_memory::persistence::event_store::InMemoryEventStore;
pub use ego_persistence_memory::persistence::repository::InMemoryRepository;
pub use ego_persistence_memory::persistence::snapshot::InMemorySnapshotStore;
pub use ego_persistence_memory::read_side::store::{paginate, InMemoryReadSideStore};
```

(el store de reservas **no** forma parte de la superficie de `ego-infrastructure` y por lo tanto no
aparece aquí). Las cuatro declaraciones `mod` (`:7-10`) y los cuatro archivos fuente se eliminan.

**Criterios**:

1. **Los módulos vaciados son privados.** `mod event_store;` (`:7`), `mod read_side_store;`
   (`:8`), `mod repository;` (`:9`), `mod snapshot;` (`:10`) — ninguno es `pub mod`. La única ruta
   pública que un consumidor puede resolver es la ruta de elemento
   (`ego_infrastructure::persistence::in_memory::InMemoryEventStore`), confirmado en los cuatro
   sitios de llamada: `examples/reference-app/src/lib.rs:432-439`,
   `examples/reference-app/src/read_side/store.rs:18`,
   `crates/infrastructure/tests/in_memory_event_store_conformance.rs:17-18` y
   `crates/infrastructure/tests/commit_publishes_atomically.rs:25`.
2. **Una re-exportación a granularidad de módulo, por lo tanto, *ensancharía* la superficie
   pública**, exponiendo por primera vez
   `ego_infrastructure::persistence::in_memory::event_store::…`. CORE-PERSIST-A eligió
   granularidad de módulo porque los módulos vaciados de `ego-domain` eran `pub mod` y decenas de
   rutas internas `super::`/`crate::` pasaban por ellos (`design.md` archivado, `:206-231`). Aquí
   no se cumple ninguna de las dos condiciones: ninguna ruta atraviesa el módulo, y la
   granularidad de elemento es a la vez el diff más estrecho y el más corto — cuatro líneas
   `pub use` redirigidas, en su sitio `:12-15`.
3. **`paginate` conserva una ruta pública.** Es una función libre, importada directamente por
   `examples/reference-app/src/read_side/store.rs:18`, y permanece en la misma línea `pub use` que
   hoy comparte con `InMemoryReadSideStore` (`:13`) — el requisito de R9 de que *cada* fila de la
   matriz resuelva, incluida la que no es una estructura.
4. **`ego-infrastructure` gana una dependencia normal** (`ego-persistence-memory`) y no pierde
   ninguna. Esa arista es `infrastructure → foundation` (AD-1), y no hace que el crate nuevo
   herede nada: aquí lo que importa es la dirección de dependencia, no la unificación de
   características — el crate nuevo no nombra ninguna de las dependencias de `ego-infrastructure`
   (AD-2, criterio 3).

### AD-7 — `examples/reference-app` conserva su superficie pública y no gana ningún shim

**Decisión** — dos archivos, ambos mínimos:

```rust
// examples/reference-app/src/read_side/store.rs — reemplaza las dos declaraciones (:150-238).
// Privado: el archivo necesita estos nombres solo para que FakeDurable{Offset,Dedup}Store
// puedan seguir envolviéndolos (:251, :282). No se republica desde aquí — mod.rs es dueño
// de la superficie del crate.
use ego_persistence_memory::read_side::{dedup::InMemoryDedupStore, offset::InMemoryOffsetStore};
```

```rust
// examples/reference-app/src/read_side/mod.rs — :36-39 pierde dos nombres, gana una línea
pub use ego_persistence_memory::read_side::{dedup::InMemoryDedupStore, offset::InMemoryOffsetStore};
pub use store::{
    FakeDurableDedupStore, FakeDurableOffsetStore, ReadSideSink, SharedReadSideStore,
};
```

`mod.rs:106-107`, `:115-116` y todos los demás cuerpos del ejemplo quedan byte-idénticos.
`store.rs` conserva `DedupKey`/`OffsetKey` solo si los fakes siguen nombrándolos — no lo hacen
(`:251,282` envuelven los tipos de store, no los alias de clave), así que esos dos alias
(`:143-148`) se mueven con las estructuras que los usan, y `HashMap`/`HashSet` abandonan el bloque
de imports de `store.rs` junto con ellos. `SharedReadSideStore` (`:33`), `ReadSideSink` (`:101`),
ambos tipos `FakeDurable*` y el módulo `#[cfg(test)]` del archivo (`:309+`) se quedan (NG-9, R3).

**Criterios**:

1. **D-6 se respeta en el sentido que importa.** «No se crea re-exportación en el ejemplo»
   significa ningún *shim de compatibilidad* — ninguna ruta preservada para un consumidor que de
   otro modo se rompería. `mod.rs:36-39` no es eso: es la superficie pública preexistente del
   propio ejemplo, y mantenerla idéntica es lo que impide que este cambio filtre en el diff un
   estrechamiento de visibilidad no relacionado (NG-7). Nada fuera del ejemplo resuelve ninguno de
   los dos nombres — un grep sobre `examples/reference-app/tests/` devuelve cero coincidencias, y
   el crate es una hoja (`reference-app/Cargo.toml:5` — `publish = false`, sin dependientes).
2. **La alternativa era eliminar ambos nombres de `mod.rs` por completo.** Rechazada: cambia la
   API pública del ejemplo dentro de un cambio cuya afirmación central es que no cambia nada
   observable, y no compra nada — `ReadSideHandles::in_memory()` sigue construyendo ambos tipos de
   cualquier manera.
3. **El ejemplo gana una dependencia normal**, `ego-persistence-memory`
   (`reference-app/Cargo.toml`), sumándose a las catorce que ya carga (`:31-64`). Está fuera del
   alcance de `verify-layers` (AD-1, criterio 4).
4. **La regla de huérfanos sigue satisfecha.** `FakeDurableOffsetStore` es un tipo local, así que
   `impl OffsetStore for FakeDurableOffsetStore` sigue siendo legal con el trait y el tipo
   envuelto ambos foráneos — exactamente la situación en la que `SharedReadSideStore`
   (`store.rs:21-33`) vive desde PROD-014A.

### AD-8 — El cambio de alcanzabilidad de D-7: qué se vuelve posible realmente, dicho una vez y con claridad

`InMemoryOperationReservationStore` pasa de `ego-testkit` (`layers.toml:34`, capa `tooling`, que
`allowed_layers` mapea a `None` — un sumidero: puede depender de cualquier cosa, y **nada puede
depender de él** en el grafo de compilación, razón por la cual
`crates/infrastructure/Cargo.toml:22-26` documenta su propia arista hacia testkit como solo-dev) a
`ego-persistence-memory` (capa `foundation`).

**Qué no cambia**: la estructura, sus campos, su `impl OperationReservationStore`
(`reservation.rs:99+`), su aritmética de lease/fencing/takeover, su estrategia de `Mutex` y la
cuestión de `is_durable()` — `OperationReservationStore` no declara tal método, así que no existe
postura de durabilidad que preservar o romper aquí. Cero comportamiento, cero contrato, cero
cambio de aserción de prueba.

**Qué cambia**: hoy sus únicos consumidores entre crates son tres archivos de prueba con
dependencia dev (`crates/transport/tests/operation_key_extractor.rs:46,260`,
`crates/service-sdk/tests/retention_worker_lifecycle.rs:22`,
`crates/service-sdk/tests/cross_tenant_reservation_isolation.rs:101`), y el grafo de capas hace
que una arista de producción hacia él sea **imposible de escribir**. Tras el movimiento, cualquier
crate `foundation`, `infrastructure`, `sdk`, `cross-cutting`, `application` o `transport` puede
tomar una dependencia normal sobre él y cablearlo en una raíz de composición con un `SystemClock`
(`crates/domain/src/time/clock.rs:33`). La compuerta deja de ser la respuesta; la persona
revisora pasa a serlo.

**Por qué el diseño lo acepta**: es la *única* implementación de `OperationReservationStore` en
todo el workspace, y su propio comentario ya reclama fidelidad de producción — «a real, full
implementation of the real production port, not a parallel model of it»
(`reservation.rs:74-78`), y el comentario del constructor añade que «production code drives an
equivalent store with `SystemClock`» (`:84-90`). Dejar la única implementación del workspace de un
puerto ya entregado dentro de un crate sumidero es exactamente el defecto de propiedad que este
cambio existe para terminar.

**Qué este diseño *no* hace**: no añade guarda, ni `#[cfg]`, ni feature flag, ni rechazo de
`Profile::Production` para este store. A diferencia de `EventStore`/`Snapshot`, cuyo valor por
defecto de `is_durable()` (`persistence-api/src/persistence/event_store.rs:54-56`,
`snapshot.rs:19-21`) le da a `require_durably_configured`
(`persistent-entity/src/profile.rs:51-63`) algo que rechazar, `OperationReservationStore` no tiene
predicado de durabilidad — así que un rechazo tendría que inventarse, e inventarlo es
comportamiento nuevo (NG-11). **La alcanzabilidad queda por lo tanto genuinamente abierta tras
este cambio, y la mitigación es la firma de quien revisa, no un mecanismo.** Se arrastra como
**OQ-2**, que reformula el ítem 1 de la propia ronda de preguntas sin responder de la propuesta.
Si la respuesta es «no», el resultado correcto es eliminar el ítem 7 de IS-2 — la rebanada S2 es
revertible de forma independiente por construcción (AD-9) — no debilitar esta decisión.

### AD-9 — Tres rebanadas, ordenadas por crecimiento de dependencias; todo estado intermedio compila en todo el workspace

`sdd-tasks` es dueña de la descomposición en tareas. Este diseño es dueño solo de los límites y su
orden.

| Rebanada | Contenido | Dependencias del crate tras la rebanada | Prueba RED |
|---|---|---|---|
| **S1 — las cuatro de infrastructure** | esqueleto del crate (`Cargo.toml`, `lib.rs`, tres `mod.rs`), miembro del workspace, entrada en `layers.toml` (AD-1); `persistence/{event_store,repository,snapshot}.rs`, `read_side/store.rs` (+ su módulo de test reubicado); `in_memory/mod.rs` redirigido a cuatro `pub use`, cuatro archivos fuente eliminados (AD-6); `ego-infrastructure` gana la dependencia | `ego-persistence-api`, `async-trait`, `serde_json`; **dev**: `tokio`, `chrono` (EC-4/EC-7) | `crates/infrastructure/tests/in_memory_reexport_identity.rs` — nombra `ego_persistence_memory::…`, que aún no existe |
| **S2 — store de reservas** | `operation/reservation.rs`; división de `testkit/src/reservation.rs` + `pub use` de módulo (AD-5); `ego-testkit` gana la dependencia; **`ego-domain` + `chrono` promovidas a dependencias normales** | añade `ego-domain`, `chrono` | `crates/testkit/tests/reservation_reexport_identity.rs` |
| **S3 — las dos de reference-app** | `read_side/{offset,dedup}.rs`; `store.rs` y `mod.rs` redirigidos (AD-7); `reference-app` gana la dependencia | sin cambios | `cargo build -p reference-app` — las dos declaraciones ya no están y las rutas nuevas deben resolver |

**Criterios**:

1. **El orden lo fuerza el cierre de dependencias, no el tamaño.** Los cuatro archivos de S1 solo
   necesitan `ego-persistence-api` (AD-2, criterio 5), así que el crate puede existir y compilar
   con un `Cargo.toml` de una sola arista. S2 es lo que hace necesaria a `ego-domain` (D-3), y
   aterrizar esa arista *con el archivo que la necesita* mantiene su razón visible en un solo diff
   en lugar de dos. S3 no necesita nada nuevo, por lo cual va última y es la más pequeña.
2. **Coincide con el propio orden de Approach de la propuesta** («infrastructure → testkit →
   reference-app») y responde al requisito de rebanado de R-4, añadiéndole el argumento del
   cierre.
3. **Todo estado intermedio es un workspace que compila.** Tras S1, las cuatro implementaciones de
   infra viven en un segundo crate y cada consumidor las resuelve sin editar mediante
   `in_memory/mod.rs`. Tras S2 ocurre lo mismo con testkit. Solo S3 edita consumidores, y su
   consumidor es un ejemplo hoja sin dependientes.
4. **El rollback sigue siendo por rebanada.** La propiedad de rollback a mitad de vuelo de la
   propuesta se cumple en cada límite: revertir S3 devuelve los dos archivos del ejemplo; revertir
   S2 reensambla `reservation.rs` (y, según AD-8, también la inalcanzabilidad impuesta por la capa);
   revertir S1 hace desaparecer el crate. Nunca se cambió estado de compuerta (AD-1), así que no
   hay nada que deshacer.
5. **El TDD estricto es satisfacible en cada rebanada.** Cada RED es un fallo de compilación por
   la razón correcta — una ruta que aún no existe — que es exactamente la forma de RED que
   `ego-rs-testing-tdd` acepta («una prueba que falla al compilar porque el tipo aún no existe es
   un RED válido»).

### AD-10 — La prueba de identidad en tiempo de compilación vive en los crates **cedentes**, no en el nuevo

**Decisión** — `crates/infrastructure/tests/in_memory_reexport_identity.rs` y
`crates/testkit/tests/reservation_reexport_identity.rs`. Cada uno lleva un testigo de identidad
por fila de la matriz de compatibilidad, con la forma de CORE-PERSIST-A: una coerción de identidad
para traits seguros ante objetos, y un testigo con cláusula `where` que carga las cotas de ambas
rutas sobre un mismo parámetro de tipo para los genéricos. Un `use` a secas es insuficiente —
compila igual de bien contra una copia redeclarada que solo comparte el nombre (IS-5, R9).

**Criterios**:

1. **Ubicarla en `crates/persistence-memory/tests/` exigiría que el crate nuevo tuviera una
   dev-dependency sobre `ego-infrastructure`.** Las aristas dev se excluyen del grafo de capas
   (`metadata.rs:122-128`, afirmado en `:188-206`; la misma excepción de la que depende
   `crates/persistence-api/Cargo.toml:27-32`), así que *pasaría* — pero arrastraría `sqlx` y toda
   la pila de OpenTelemetry a la compilación de pruebas del crate nuevo, y convertiría al crate de
   nombre más limpio del workspace en su consumidor más pesado en `cargo metadata`. No compensa.
2. **La promesa pertenece a quien la hace.** `ego-infrastructure` es el crate que le dice al mundo
   «`ego_infrastructure::persistence::in_memory::InMemoryEventStore` sigue resolviendo»; la prueba
   que lo demuestra pertenece junto a esa afirmación, donde quien edite `in_memory/mod.rs` en el
   futuro tropiece con ella.
3. **`reference-app` no necesita tal archivo.** No publica promesa de compatibilidad alguna
   (AD-7), así que su prueba es `cargo build -p reference-app` — que ya está en el
   `verify.build_command` de `openspec/config.yaml` (`cargo build --workspace`).

---

## Puntos de integración

| Frontera | Dirección | Mecanismo | Verificado en |
|---|---|---|---|
| `ego-persistence-memory` → `ego-persistence-api` | nueva, unidireccional | dependencia `path`; puertos resueltos directamente, nunca vía `ego-domain` | AD-2, AD-4 |
| `ego-persistence-memory` → `ego-domain` | nueva, unidireccional, **un elemento** | dependencia `path` solo para `Clock` | `reservation.rs:20,80`; AD-2 |
| `ego-persistence-memory` → cualquier otro crate del workspace | **ninguna** | no existe dependencia `path` | AD-2, criterio 3; R11 |
| `ego-infrastructure` → `ego-persistence-memory` | nueva, unidireccional | dependencia `path` + cuatro re-exportaciones de elemento | AD-6 |
| `ego-testkit` → `ego-persistence-memory` | nueva, unidireccional | dependencia `path` + un `pub use` a nivel de módulo | AD-5 |
| `reference-app` → `ego-persistence-memory` | nueva, unidireccional | dependencia `path` + dos imports ordinarios | AD-7 |
| cada consumidor existente → elementos movidos | **sin cambio** | resueltos mediante las re-exportaciones de los crates cedentes | tabla siguiente; R9 |
| `layers.toml` → `verify-layers` | entrante | una entrada nueva, cargador existente | `layers.rs:148-158`; AD-1 |
| `allowed_layers` → `check_direction` | **ninguna** | ningún brazo de la matriz cambia | `layers.rs:74-92`; AD-1 |
| comportamiento en ejecución | **ninguno** | nada se ejecuta distinto | D-4, R5 |

**La matriz de compatibilidad, reexpuesta con las rutas de AD-3 y los mecanismos de AD-5/AD-6/AD-7:**

| Ruta antigua (debe resolver, sin editar) | Ruta canónica nueva | Sitio de re-exportación |
|---|---|---|
| `ego_infrastructure::persistence::in_memory::InMemoryEventStore` | `ego_persistence_memory::persistence::event_store::InMemoryEventStore` | `pub use` de elemento en `in_memory/mod.rs` |
| `ego_infrastructure::persistence::in_memory::InMemoryRepository` | `ego_persistence_memory::persistence::repository::InMemoryRepository` | igual |
| `ego_infrastructure::persistence::in_memory::InMemorySnapshotStore` | `ego_persistence_memory::persistence::snapshot::InMemorySnapshotStore` | igual |
| `ego_infrastructure::persistence::in_memory::{InMemoryReadSideStore, paginate}` | `ego_persistence_memory::read_side::store::{InMemoryReadSideStore, paginate}` | igual |
| `ego_testkit::InMemoryOperationReservationStore` | `ego_persistence_memory::operation::reservation::InMemoryOperationReservationStore` | `pub use` en `testkit/src/reservation.rs`; `lib.rs:50` sin cambio |
| `crate::reservation::…` dentro de los dos módulos de prueba de testkit (`:378`, `:525`) | igual | el mismo `pub use` — EC-3 |
| `InMemoryEventStoreUnitOfWork` | `ego_persistence_memory::persistence::event_store::InMemoryEventStoreUnitOfWork` | **no requerida** — privada, alcanzable solo como `Box<dyn EventStoreUnitOfWork<E>>` desde `begin()` |
| `InMemoryOffsetStore` / `InMemoryDedupStore` de reference-app | `ego_persistence_memory::read_side::{offset,dedup}::…` | **ninguna** — imports actualizados en el sitio (AD-7) |
| `persistent_entity::persistence::{InMemoryEventStore, InMemorySnapshotStore}` | — | **sin cambio**: duplicados propios de `persistent-entity` (`persistence.rs:571,733`), no movidos (D-9, EC-6) |

Consumidores aguas abajo confirmados que deben compilar con fuente byte-idéntica:
`crates/infrastructure/tests/in_memory_event_store_conformance.rs:17-18`,
`crates/infrastructure/tests/commit_publishes_atomically.rs:25`,
`examples/reference-app/src/lib.rs:432-439`,
`crates/transport/tests/operation_key_extractor.rs:46,260`,
`crates/service-sdk/tests/retention_worker_lifecycle.rs:22`,
`crates/service-sdk/tests/cross_tenant_reservation_isolation.rs:101`.
`crates/persistent-entity/src/builder.rs` **no** está en esta lista (EC-6).

## Estrategia de pruebas

TDD estricto (`openspec/config.yaml` → `apply.tdd: true`). El RED de cada rebanada es un fallo de
compilación que nombra una ruta aún inexistente (AD-9), lo que `ego-rs-testing-tdd` acepta como
RED válido. Este cambio no escribe comportamiento nuevo, así que no se gana ninguna prueba de
comportamiento nueva — las aserciones que importan ya existen y deben seguir pasando **sin
modificar**.

| Nivel | Ubicación | Qué demuestra |
|---|---|---|
| Tiempo de compilación (principal) | `crates/infrastructure/tests/in_memory_reexport_identity.rs`, `crates/testkit/tests/reservation_reexport_identity.rs` | **IS-5 / R9** sobre la matriz completa, nunca una muestra. Testigos de identidad, no `use` a secas (AD-10) |
| Unitaria reubicada | módulo `#[cfg(test)]` movido de `read_side/store.rs` | **D-4 / R5**: se mueve textual con su archivo. Conteo y texto de aserciones idénticos antes y después — una aserción cambiada es señal de deriva, no una limpieza (EC-4) |
| Unitaria retenida | `testkit/src/reservation.rs:370-512`, `:514-…` | **D-8 / R16**: `TestClock` y ambas suites se quedan y manejan el store re-exportado vía `super::` — byte-idénticas (EC-3, AD-5) |
| Unitaria retenida | `examples/reference-app/src/read_side/store.rs:309+` | la suite propia del ejemplo sigue ejercitando `SharedReadSideStore`, `ReadSideSink` y ambos tipos `FakeDurable*` (NG-8, R3) |
| Integración (intacta) | `crates/infrastructure/tests/{in_memory_event_store_conformance,commit_publishes_atomically}.rs` | **R9 / R14**: el arnés de conformidad conserva su forma y su hogar actuales; compila mediante la re-exportación con fuente byte-idéntica |
| Integración (intacta) | `crates/persistent-entity/src/builder.rs:768,793`, `profile.rs:99-117` | **R6**: `presence_alone_is_not_durability` y ambas pruebas `try_build_rejects_explicit_in_memory_*` pasan sin modificar. Ejercitan los tipos propios de `persistent-entity` (EC-6), así que se cumplen trivialmente — y se cumplirían igual, ya que ningún tipo movido declara `is_durable()` (AD-4) |
| Compuerta | `cargo run -p xtask -- verify-layers` | **R11**: mapeado (FR-001), cada arista permitida (FR-002), sin ciclos (FR-003), compilación aislada (FR-005) — **sin edición de matriz** (AD-1) |
| Workspace | `cargo build --workspace`, `cargo test --workspace` | cero fallos nuevos, cero aserciones cambiadas; cubre a cada consumidor del árbol, incluidos los que ninguna prueba nombra (R-7) |

Seis propiedades son **propiedades del diff** — verificadas leyendo el cambio, no con una prueba:

- **R5** — cada cuerpo movido es textualmente idéntico salvo la ruta de módulo y la tabla de
  imports de AD-4.
- **R7 / R11** — el `Cargo.toml` nuevo nombra exactamente el conjunto de AD-2; ningún token
  `sqlx`, Postgres, Stoolap, HTTP ni Kafka aparece bajo `crates/persistence-memory/`.
- **AD-2, criterio 4** — `rg 'ego_domain::' crates/persistence-memory/src` devuelve exactamente
  una línea.
- **R12 / R13** — `crates/runtime/`, `crates/effect-store/`, `crates/persistence/` y todo archivo
  `.sql`/de migración están ausentes de la lista de archivos.
- **R15** — `crates/persistence-api/src/**` está ausente de la lista de archivos.
- **R2 / R10** — el conteo global del workspace de bloques `impl <Puerto> for` por puerto movido no
  cambia; las únicas declaraciones no canónicas sobrevivientes son los dos duplicados de
  `persistent-entity` y los fakes de prueba declarados.

## Matriz de amenazas

N/A — no hay enrutamiento, comando de shell, subproceso, automatización de VCS/PR, clasificación
de archivos ejecutables ni frontera de integración de procesos. Este cambio mueve archivos fuente
de Rust entre cuatro crates y agrega una línea a una tabla TOML.

`ego-rs-security` es aplicable solo para confirmar que queda intacto: cero texto SQL, cero
construcción de consultas, cero ruta de autenticación, cero verificación JWT y cero comprobación
de `CrossTenantPermit` aparecen en el diff. Dos comportamientos de aislamiento de tenant se
reubican y deben reubicarse **textualmente**, algo que R5 ya fija: la guarda fail-closed de tenant
vacío de `paginate` (`read_side_store.rs:111-115`) y el almacenamiento con clave de
`resolve_tenant` en `event_store.rs`, `repository.rs` y `snapshot.rs` (`snapshot.rs:38-39,49-50`).
`resolve_tenant` en sí no se mueve — se queda en `ego-persistence-api`
(`persistence/tenant.rs`), donde CORE-PERSIST-A lo puso.

Un cambio de frontera de acceso es real y no se oculta aquí: **AD-8**. Es un cambio de
alcanzabilidad, no de comportamiento del código, y es el único ítem de este diseño que necesita
una respuesta humana (OQ-2).

## Migración / Despliegue

**No se requiere migración.** No existe dato, esquema, archivo de migración ni estado persistido
en ninguna dirección — este cambio no escribe nada en ejecución. Sin feature flag y sin despliegue
por fases: el workspace compila con la disposición nueva o no compila.

El rollback es el de la propuesta, sin cambios, y está disponible en cada uno de los tres límites
de AD-9: eliminar `crates/persistence-memory/`, restaurar los cuatro archivos de
`ego-infrastructure` y `in_memory/mod.rs`, reensamblar `crates/testkit/src/reservation.rs`,
restaurar las dos declaraciones de la app de referencia y sus dos sitios de import, quitar la
entrada de `layers.toml` y el miembro del workspace, y eliminar las tres aristas de `Cargo.toml`.
`xtask/src/layers.rs` nunca se abrió, así que no hay estado de compuerta que deshacer.

## Trazabilidad

| Ítem de propuesta / explore | Resuelto por | Nota |
|---|---|---|
| D-1, IS-1 | AD-1, AD-2, AD-3 | crate en `crates/persistence-memory/`, paquete `ego-persistence-memory` |
| **D-2** | **AD-1** | el mapeo vive en `layers.toml`, no en `layers.rs`; una línea; cada arista ya permitida; `layers.rs` intacto — **confirmado contra el código, no asumido** |
| **D-3, R-3** | **AD-2 (criterios 1, 4, 5)** | lista exacta de dependencias derivada por archivo; la arista `ego-domain` es `Clock` y solo `Clock`; no existe tercera arista, confirmado para las siete implementaciones |
| D-4, R5 | **AD-4**, EC-1, EC-2 | la reescritura enumerada por archivo; `operation::` no aplana como lo hace `ego_domain::operation`; existe una ruta en línea que la redacción de D-4 no cubre (→ OQ-1) |
| D-5, IS-3, R9 | **AD-6**, **AD-5**, EC-3 | granularidad de elemento para infrastructure (módulos privados), `pub use` a nivel de módulo para testkit (dos módulos de prueba resuelven vía `super::`) |
| D-6, IS-4, R8 | **AD-7**, EC-5 | dos archivos, no uno; superficie pública del ejemplo preservada; sin shim de compatibilidad |
| **D-7, R-1** | **AD-8**, **OQ-2** | el cambio de alcanzabilidad expuesto por completo, con qué cambia y qué no, y por qué ningún mecanismo sustituye a quien revisa |
| D-8, R16 | AD-5 | `TestClock` y ambas suites colocadas se quedan, byte-idénticas |
| D-9, KD-5, KD-6, NG-1, NG-2, R17, F-5, F-6 | — | `crates/persistent-entity/` no aparece en ninguna lista de archivos aquí; EC-6 confirma que `builder.rs` no necesita nada |
| D-10, NG-6, R12, R18, F-1 | — | `crates/runtime/` y `crates/effect-store/` no aparecen en ninguna lista de archivos aquí; el EFFECT STORE BLOCKER ANALYSIS de `explore.md` se consume tal cual |
| D-11, R6 | AD-4, Pruebas | ningún tipo movido declara `is_durable()`; las pruebas que rechazan ejercitan los tipos propios de `persistent-entity` (EC-6) y quedan intactas en cualquier caso |
| D-12, KD-1, NG-10, R4 | AD-3 | `ProjectionStateStore` no recibe módulo, ni archivo, ni `todo!()` — el árbol de arriba no tiene fila para él |
| IS-2 | AD-3, AD-4, AD-9 | las siete, cada una mapeada a un archivo destino y a una rebanada |
| IS-5, R9 | **AD-10** | testigos de identidad en los crates cedentes, matriz completa, sin muestreo |
| NG-8, R3 | AD-7 | los `FakeDurable*` se quedan en el ejemplo, byte-idénticos, envolviendo los tipos ahora externos |
| NG-11, R7 | AD-2, AD-3 | sin re-exportación en la raíz, sin elemento nuevo, sin token de backend, sin documentación forzada por `#![deny(missing_docs)]` |
| NG-12 | AD-6, AD-7, AD-9 | cambian exactamente cuatro `Cargo.toml`: el crate nuevo, `ego-infrastructure`, `ego-testkit`, `reference-app`, más la lista de miembros de la raíz |
| R-2 | AD-4, propiedades del diff en Pruebas | «textual» es una comparación de texto, y las ediciones permitidas son una tabla enumerada en lugar de un juicio |
| **R-4** | **AD-9** | tres rebanadas por crate de origen, cada una compilando y revertible de forma independiente; se añade el argumento del cierre al orden de la propuesta |
| R-5 | — | el reencuadre de `persistence-api-surface` corresponde a `sdd-spec`; este diseño no edita ninguna spec ni toca ningún archivo de `crates/persistence-api/` |
| R-6 | AD-1 (nota de cierre) | el comentario obsoleto de `layers.toml:6` queda nombrado, no corregido (NG-7) |
| R-7 | AD-10, Pruebas | prueba en tiempo de compilación sobre la matriz completa más `cargo build --workspace` |
| KD-4, NG-4, R14, F-4 | Pruebas | no se agrega, extiende ni generaliza ningún arnés |
| `config.yaml` «sequence diagrams» | Enfoque técnico | N/A explícito — cero rutas de llamada agregadas, eliminadas o reordenadas |
| `config.yaml` «no circular dependencies» | Grafo de dependencias | dos aristas unidireccionales hacia un crate que no nombra ninguna dependencia propia del workspace |
| `config.yaml` «decisions with rationale» | AD-1..AD-10 | cada una lleva criterios y, donde existía, la alternativa rechazada |

## Preguntas abiertas

- [ ] **OQ-1 — La redacción de D-4 no cubre `reservation.rs:70`.** Es una ruta calificada en línea,
      no una línea `use` (EC-2). AD-5 la reescribe, lo que mantiene la superficie `ego_domain::`
      del crate en exactamente una línea y nombra el mismo elemento en cualquier caso. Confirmar la
      enmienda de una cláusula («líneas `use`» → «expresiones de ruta que nombran un elemento de
      puerto reubicado») antes de que `sdd-tasks` planifique S2. **No bloqueante**: dejar `:70`
      byte-idéntica también compila; solo el criterio 4 de AD-2 pierde su `grep` limpio.
- [ ] **OQ-2 — El cambio de alcanzabilidad de D-7 sigue sin respuesta** (ronda de preguntas de la
      propuesta, ítem 1). AD-8 expone exactamente qué se vuelve posible y confirma que ningún
      mecanismo puede sustituir la firma sin inventar comportamiento (NG-11). **Bloqueante solo
      para la rebanada S2** — S1 y S3 no se ven afectadas y pueden avanzar. Un «no» elimina el ítem
      7 de IS-2; no debilita D-7.
- [ ] **OQ-3 — AD-3 refina el árbol de `explore.md`** (`read_side/read_side_store.rs` →
      `read_side/store.rs`), lo que desplaza una fila de su COMPATIBILITY REEXPORT MATRIX. La
      matriz reexpuesta en Puntos de integración es la autoridad para `sdd-tasks`. Se señala para
      que el desplazamiento quede como decisión registrada y no como una discrepancia que alguien
      descubra después. **No bloqueante.**
