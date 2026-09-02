# Propuesta: CORE-PERSIST-B — Adaptador de Persistencia en Memoria de Primera Clase

> Compañero de revisión en español. Fuente canónica: `proposal.md` (identificadores 1:1:
> D-1..D-12, NG-1..NG-12, R1..R18, KD-1..KD-4, F-1..F-4).
>
> Base factual: `explore.md` en esta misma carpeta. Su inventario, MOVE MATRIX, COMPATIBILITY
> REEXPORT MATRIX y bloque READINESS se consumen tal cual, no se vuelven a derivar.

## Objetivo

Dar al workspace **un solo** crate adaptador de persistencia en memoria. Siete implementaciones
candidatas canónicas de puertos de `ego-persistence-api` se reubican de forma literal en un nuevo
crate `ego-persistence-memory`, con un reexport de compatibilidad en cada ruta que un consumidor
resuelve hoy. Cero comportamiento nuevo, cero cambio de contrato, cero trabajo sobre Postgres.

## Intención

**El problema es de propiedad, no de comportamiento.** Tras CORE-PERSIST-A cada puerto de
persistencia propiedad del dominio tiene exactamente un crate dueño, pero sus implementaciones en
memoria no. Están dispersas en tres crates de tres capas distintas, y la capa en la que cada una
vive es un accidente de quién la necesitó primero, no una decisión que alguien haya tomado:

- `ego-infrastructure` (capa `infrastructure`) posee cuatro de ellas, en un crate cuyo `Cargo.toml`
  además arrastra `sqlx`, `ego-application` y el stack de OpenTelemetry, nada de lo cual importa el
  submódulo `in_memory` (`explore.md`, DEPENDENCY GRAPH).
- `ego-testkit` (capa `tooling`, un **sumidero**) posee `InMemoryOperationReservationStore`, la
  única implementación de `OperationReservationStore` en todo el workspace y, según su propio
  comentario de documentación (`crates/testkit/src/reservation.rs:74-78`), "una implementación real
  y completa del puerto de producción real, no un modelo paralelo de él". Como `tooling` es un
  sumidero, **ningún crate de producción puede alcanzarla.**
- `examples/reference-app` posee `InMemoryOffsetStore` e `InMemoryDedupStore`, las únicas
  implementaciones de `OffsetStore` y `DedupStore` que existen, hecho que el propio ejemplo admite
  en un comentario (`store.rs:150-151`, `store.rs:196-197`). **Un ejemplo actúa como dueño de la
  infraestructura del workspace.**

El costo observable: quien hoy necesita un `OffsetStore` en memoria debe depender de un crate de
ejemplo o escribir el decimoquinto doble de prueba. Un framework que publica ocho puertos y esconde
las únicas implementaciones de tres de ellos en un crate de pruebas y en un ejemplo no ofrece una
superficie de persistencia: ofrece una búsqueda del tesoro.

**Este cambio es puramente estructural.** Cada implementación movida conserva su cuerpo byte a
byte; solo cambian su ruta de módulo y sus líneas `use` (D-4). Nada es observable en tiempo de
ejecución.

## Decisiones activas

| ID | Decisión | Fundamento / evidencia |
|----|----------|------------------------|
| D-1 | **Nuevo crate `ego-persistence-memory` en `crates/persistence-memory/`.** Nombre confirmado, no simplemente heredado | Respeta la convención paquete `ego-*` / directorio sin prefijo que ya fijó `ego-persistence-api` en `crates/persistence-api/`, y se mantiene inequívocamente distinto del `ego-persistence` existente (adaptadores Postgres). La propia D-1 de CORE-PERSIST-A anticipó este nombre exacto: "`persistence-postgres` / `persistence-memory` renames are CORE-PERSIST-B/C's job" (`proposal.md:43` archivado) |
| D-2 | **La clasificación de capa es `foundation`, no `domain`.** `layers.toml` incorpora `"ego-persistence-memory" = "foundation"` y **`xtask/src/layers.rs` no se modifica en absoluto** | Tres razones. (a) **Honestidad**: un adaptador en memoria es un adaptador; en términos hexagonales los puertos son artefactos de dominio y las implementaciones no. (b) **Contención**: `foundation → domain` ya está permitido (`layers.rs:77`), así que no hace falta relajar ninguna compuerta; mapearlo a `domain` aprovecharía la arista propia de D-4 de CORE-PERSIST-A y la ampliaría de "un crate de dominio puede alcanzar un crate de *puertos*" a "un crate de dominio puede alcanzar un *adaptador*", legalizando `ego-domain → ego-persistence-memory`. (c) **La alcanzabilidad basta**: la capa de cada consumidor real ya permite depender de `foundation` — `infrastructure` (`layers.rs:80-86`), `sdk` (`:87`), `foundation` (`:77`), `tooling` (sumidero, `:89`). `domain` y `application` no pueden alcanzarlo, que es el resultado correcto para un adaptador |
| D-3 | **El conjunto de dependencias es `ego-persistence-api` **más `ego-domain`**, y la segunda arista se nombra en lugar de darse por supuesta.** `InMemoryOperationReservationStore` mantiene un `Arc<dyn Clock>` (`crates/testkit/src/reservation.rs:80`), y `Clock` **no** fue reubicado por CORE-PERSIST-A: vive en `crates/domain/src/time/clock.rs:24` | El DEPENDENCY GRAPH de `explore.md` lista `ego_domain::Clock` entre los imports de `reservation.rs`, pero su resumen de reglas de dependencia se lee como "solo `ego-persistence-api`"; esta propuesta cierra ese hueco de forma explícita. `Clock` es una abstracción de valor de dominio, de modo que la arista cae en el caso "crate de valores de dominio inevitable", no en una ampliación de alcance. Es legal (`foundation → domain`, `layers.rs:77`) y acíclica (`memory → domain → persistence-api`). **Se rechaza reubicar `Clock` a `ego-persistence-api`**: ampliaría una superficie de contrato ya entregada y archivada para dar servicio a un movimiento, que es justamente lo que este cambio tiene prohibido hacer. `design.md` fija la lista final de imports |
| D-4 | **La reubicación es literal en el cuerpo y reescrita solo en las líneas `use`.** Cuerpos de structs, impls de traits, comentarios de documentación, estrategia de bloqueo y lógica de resolución de tenant se mueven sin editar; la única edición permitida es reescribir `use ego_domain::persistence::…` a `use ego_persistence_api::persistence::…` allí donde el ítem ahora resuelve directamente | Refleja la D-6 de CORE-PERSIST-A y viene forzado por D-3: el nuevo crate resuelve los puertos directamente desde `ego-persistence-api` en lugar de hacerlo a través de la capa de reexports de `ego-domain`. La reescritura toca solo líneas de import y no cambia ningún ítem resuelto: ambas rutas nombran los mismos tipos por construcción (spec `persistence-api-surface`, "Old Path Resolves To The Same Item") |
| D-5 | **Toda ruta antigua sigue resolviendo, mediante un reexport de compatibilidad en el crate que se vacía.** `crates/infrastructure/src/persistence/in_memory/mod.rs:12-15` pasa a ser `pub use ego_persistence_memory::…`; `crates/testkit/src/lib.rs` hace lo propio con el store de reservas | Refleja la D-5 de CORE-PERSIST-A y es lo que hace este cambio reversible a mitad de camino (Plan de reversión). Consumidores confirmados que deben compilar sin edición: `crates/infrastructure/tests/in_memory_event_store_conformance.rs:17-18`, `crates/infrastructure/tests/commit_publishes_atomically.rs`, `examples/reference-app/src/lib.rs:432-439`, `crates/transport/tests/operation_key_extractor.rs`, `crates/service-sdk/tests/{retention_worker_lifecycle,cross_tenant_reservation_isolation}.rs` (COMPATIBILITY REEXPORT MATRIX de `explore.md`) |
| D-6 | **`examples/reference-app` deja de ser dueño de infraestructura.** `InMemoryOffsetStore` e `InMemoryDedupStore` salen del ejemplo; sus propias sentencias `use` se actualizan al nuevo crate. **No se crea ningún reexport en el ejemplo** | El ejemplo es una hoja sin consumidores externos, así que una capa de compatibilidad allí sería peso muerto: es el único punto donde actualizar imports resulta más barato y más claro que reexportar. Todo lo genuinamente específico del ejemplo permanece intacto: `SharedReadSideStore` (contiene lógica de tenant/tag propia de reference-app, `store.rs:66-78`), `ReadSideSink` y ambos stores `Fake*Durable*` |
| D-7 | **Sacar `InMemoryOperationReservationStore` de `ego-testkit` lo vuelve alcanzable desde producción por primera vez. Esta propuesta lo trata como una decisión que requiere aprobación explícita, no como un efecto colateral** | `ego-testkit` es capa `tooling`, un sumidero del que ningún crate de producción puede depender (`layers.toml:13`). Hoy los únicos consumidores del store fuera de su crate son archivos de prueba vía dev-dependency. Tras el movimiento queda en `foundation` y cualquier crate `foundation`/`infrastructure`/`sdk` puede depender de él. **Ningún comportamiento de código cambia**: mismo struct, mismo impl de trait, mismo valor por defecto de `is_durable()`. Lo que cambia es quién tiene permitido cablearlo. Se justifica porque es la *única* implementación de `OperationReservationStore` del workspace y su propio comentario ya reclama fidelidad de producción (`reservation.rs:74-78`), pero el alcance ampliado se declara aquí para que un revisor lo apruebe deliberadamente |
| D-8 | **`TestClock` y las pruebas colocalizadas de reservas permanecen en `ego-testkit`.** Solo se mueve el store | `TestClock` (`crates/testkit/src/reservation.rs:27`) es un doble de prueba determinista y pertenece por construcción al crate de tooling (NG-8, R16). Sus pruebas colocalizadas (`reservation.rs:528+`) accionan el store *a través* de `TestClock`, de modo que ejercitan sin cambios el store reexportado desde testkit. `design.md` decide el mecanismo exacto de separación; la frontera —los dobles se quedan, las implementaciones reales se mueven— queda fijada aquí |
| D-9 | **Los dos duplicados de `persistent-entity` siguen bifurcados.** `InMemoryEventStore` (`crates/persistent-entity/src/persistence.rs:571`) e `InMemorySnapshotStore` (`:733`) no se mueven, no se fusionan y no se corrigen | Fusionar cualquiera de los dos exige una decisión de comportamiento, prohibida en este cambio. El event store lleva una capacidad aditiva `with_version_offset()` (`persistence.rs:600-611,719-727`) de la que depende `crates/persistent-entity/tests/in_memory_version_offset_parity.rs:15,22-23`: consolidar la eliminaría o la añadiría al crate canónico, y ambas cosas son comportamiento nuevo. El snapshot store es peor: es un **defecto confirmado de aislamiento de tenant** (KD-5). Deuda nombrada, no resolución silenciosa (NG-1, NG-2, R17) |
| D-10 | **La frontera del effect store queda cerrada.** `InMemoryEffectStore` (`crates/runtime/src/effects/store.rs:531`) no se mueve y `ego-runtime` no se toca | Sus puertos (`EffectStateStore` `:238`, `EffectDedupStore` `:418`, `RetentionMaintenance` `:474`) son propiedad de `ego-runtime`, no de `ego-persistence-api`; la D-9 de CORE-PERSIST-A difirió exactamente esto. Mover la implementación sin sus puertos la deja sin implementar nada; moverla dejando los puertos en su sitio fuerza `ego-persistence-memory → ego-runtime`, invirtiendo la dirección hacia un crate que a su vez depende de `persistent-entity` (`crates/runtime/Cargo.toml:7,11`). La reubicación de puertos debe llegar primero, como cambio propio: **CORE-PERSIST-E** (F-1, R18) |
| D-11 | **La semántica de durabilidad no se toca, por lo que el rechazo en producción se preserva.** Ningún tipo movido gana, pierde ni sobrescribe `is_durable()` | `EventStore::is_durable()` y `Snapshot::is_durable()` valen `false` por defecto (`crates/persistence-api/src/persistence/event_store.rs:54-56`, `snapshot.rs:19-21`) y ninguna implementación movida los sobrescribe. `require_durably_configured` de `Profile::Production` (`crates/persistent-entity/src/profile.rs:51-63`) rechaza según `is_durable()`, no según presencia, fijado por `presence_alone_is_not_durability` (`profile.rs:99-117`) y por dos pruebas del builder (`crates/persistent-entity/src/builder.rs:764-783,788-805`). Un movimiento puro no puede alterar un valor por defecto que no toca, así que esos rechazos se disparan igual tras el cambio (R6) |
| D-12 | **Lo ausente sigue ausente; los dobles siguen siendo dobles.** `ProjectionStateStore` no gana implementación (KD-1) y ningún doble local de `#[cfg(test)]` se promueve | `ProjectionStateStore` tiene cero implementaciones en todo el workspace, confirmado dos veces (`crates/persistence-api/src/read_side/projection_state.rs:27`; `verify-report.md:64` de CORE-PERSIST-A). Implementarlo aquí convertiría este cambio en una funcionalidad disfrazada de movimiento. Igualmente, los ~150 dobles locales de prueba y los dos stores `Fake*Durable*` que mienten sobre `is_durable()` (`store.rs:251,282`) quedan excluidos por construcción (R3, R4, R16) |

## Compuerta de atomicidad

**Ejecutada.** Un único movimiento indivisible: siete implementaciones que comparten un crate
destino, una decisión de capa (D-2), una decisión de dirección de dependencias (D-3) y una
estrategia de reexport por crate vaciado (D-5, D-6). Reubicar un subconjunto dejaría la superficie
del adaptador en memoria partida en dos crates, que es exactamente la condición que este cambio
existe para terminar.

Explícitamente **FUERA**, cada uno por ser una decisión independiente y no una pieza faltante de
esta: una nueva implementación de store · cualquier cambio de contrato o de firma de trait ·
la reubicación de puertos de efectos · la consolidación de PostgreSQL · un framework de pruebas de
conformidad · cualquier corrección de defecto · cualquier comportamiento nuevo en tiempo de
ejecución.

**ATOMICIDAD: PASS**, coincidiendo con el ATOMICITY VERDICT de `explore.md` y su
`RECOMMENDATION: PROCEED`. Ningún contrato de CORE-PERSIST-A requiere modificación (verificado
contra `openspec/specs/persistence-api-surface/spec.md`); ver el riesgo R-5 para la única salvedad
de redacción de spec, que es una cuestión de alcance documental, no un cambio de contrato.

## Alcance

**La frontera de un vistazo**

| | |
|---|---|
| **CORE-PERSIST-B incluye** | Nuevo crate `ego-persistence-memory` · 7 implementaciones reubicadas literalmente · reexports de compatibilidad en cada ruta antigua · entrada en `layers.toml` · imports de reference-app actualizados |
| **CORE-PERSIST-B excluye** | Todo duplicado en `persistent-entity` · todo effect store · todo trabajo de PostgreSQL · todo arnés de conformidad · toda corrección de defecto · todo doble de prueba |

### Dentro del alcance

- **IS-1** — El nuevo crate (D-1), mapeado a `foundation` en `layers.toml` (D-2), dependiendo solo
  de `ego-persistence-api` y `ego-domain` más crates externos (D-3).
- **IS-2** — Reubicación literal de las siete candidatas canónicas (D-4), todas las filas marcadas
  `Move allowed: YES` en la MOVE MATRIX de `explore.md`:
  1. `InMemoryEventStore` + `InMemoryEventStoreUnitOfWork` — `crates/infrastructure/src/persistence/in_memory/event_store.rs:89,214`
  2. `InMemoryRepository` — `.../in_memory/repository.rs:11`
  3. `InMemorySnapshotStore` (la correcta respecto a tenant) — `.../in_memory/snapshot.rs:12`
  4. `InMemoryReadSideStore` + `paginate` — `.../in_memory/read_side_store.rs:24,105`
  5. `InMemoryOffsetStore` — `examples/reference-app/src/read_side/store.rs:153`
  6. `InMemoryDedupStore` — `examples/reference-app/src/read_side/store.rs:199`
  7. `InMemoryOperationReservationStore` — `crates/testkit/src/reservation.rs:79`
- **IS-3** — Reexports de compatibilidad en `ego-infrastructure` y `ego-testkit` en cada ruta de la
  COMPATIBILITY REEXPORT MATRIX de `explore.md` (D-5).
- **IS-4** — Sentencias `use` propias de `examples/reference-app` actualizadas al nuevo crate (D-6).
- **IS-5** — Una prueba en tiempo de compilación de que cada ruta antigua resuelve al *mismo* ítem
  y no a una copia homónima.
- **IS-6** — Deltas de spec según la sección de Capacidades.

### Fuera del alcance — No-objetivos

Cada punto es un **no-objetivo con razón declarada**, no una omisión.

- **NG-1 — No se corrige ningún defecto, incluido el confirmado.** El `InMemorySnapshotStore` de
  `crates/persistent-entity/src/persistence.rs:733` recibe `_tenant_id: Option<&str>` y nunca lo lee
  (`:746-765`), indexando snapshots solo por `stream_id` (`:734`): dos tenants que escriban el mismo
  `aggregate_id` se sobrescriben en silencio. **Razón**: corregirlo aquí sería un cambio de
  corrección infiltrado dentro de un movimiento, y llegaría sin las pruebas dedicadas ni la revisión
  de radio de impacto que merece una corrección de aislamiento de tenant. Se arrastra como
  **KD-5 → F-5** (R17).
- **NG-2 — El duplicado `InMemoryEventStore`/`StagingUnitOfWork` de `persistent-entity` no se
  consolida.** **Razón**: su capacidad `with_version_offset()` es aditiva respecto de la
  implementación canónica; fusionarlas añade comportamiento de un lado o lo elimina del otro (D-9).
  Se arrastra como **KD-6 → F-6** (R17).
- **NG-3 — Ninguna consolidación de PostgreSQL.** Ningún cambio de SQL, migración, índice,
  transacción, reintento o pool de conexiones. **Razón**: otro backend, otro perfil de riesgo y otra
  audiencia de revisión; se difiere a **CORE-PERSIST-C** (R13).
- **NG-4 — No se construye ni se extiende ningún framework de pruebas de conformidad.**
  `Repository`, `Snapshot`, `OffsetStore` y `DedupStore` siguen sin arnés (KD-4). **Razón**: diseñar
  una superficie de conformidad es una capacidad, no una reubicación; se difiere a
  **CORE-PERSIST-D** (R14).
- **NG-5 — No se rediseña ninguna firma de puerto o contrato en `ego-persistence-api`.** Ningún
  método, cota, supertrait, cuerpo por defecto, forma async/sync ni propiedad de object-safety
  cambia. **Razón**: CORE-PERSIST-A entregó esa superficie; reabrirla dentro de un movimiento haría
  el diff irrevisable (R15).
- **NG-6 — No se resuelve la frontera de propiedad del effect store.** `InMemoryEffectStore`,
  `EffectStateStore`, `EffectDedupStore` y `RetentionMaintenance` permanecen en `ego-runtime`.
  **Razón**: bloqueado por la D-9 de CORE-PERSIST-A; los puertos deben reubicarse primero, lo que es
  una decisión de arquitectura aparte. Se nombra **CORE-PERSIST-E** (D-10, F-1, R18).
- **NG-7 — Ningún renombrado cosmético, reorganización o "mejora" ajena al movimiento.** **Razón**:
  cada edición no relacionada con el movimiento dentro de este diff es un lugar donde puede
  esconderse una desviación semántica (D-4).
- **NG-8 — Ningún doble de prueba especializado se promueve a adaptador.** `FakeDurableOffsetStore`
  (`store.rs:251`), `FakeDurableDedupStore` (`store.rs:282`), `TestClock` y todo doble local de
  `#[cfg(test)]` se quedan exactamente donde están. **Razón**: los stores `Fake*Durable*`
  sobrescriben `is_durable()` para devolver `true` sin serlo; su propio comentario dice "Never wire
  this into a deployment" (`store.rs:240-249`). Promover uno metería una mentira sobre durabilidad
  dentro de un crate adaptador publicado (R3, R16).
- **NG-9 — La aplicación de referencia no se trata como dueña canónica de infraestructura.**
  `SharedReadSideStore`, `ReadSideSink` y ambos dobles permanecen en el ejemplo, intactos.
  **Razón**: contienen lógica específica de reference-app (`store.rs:66-78`), no una implementación
  genérica de contrato (D-6, R8).
- **NG-10 — `ProjectionStateStore` no se implementa.** Se queda con cero implementaciones.
  **Razón**: la transparencia es preferible a un stub cómodo; una implementación falsa escondería un
  hueco real detrás de un build en verde. Se arrastra como **KD-1** (D-12, R4).
- **NG-11 — No se añade ninguna capacidad, trait, método o tipo que no exista hoy.** **Razón**: este
  cambio mueve código; no escribe ninguno.
- **NG-12 — Ningún `Cargo.toml` fuera del nuevo crate, los dos crates vaciados, la aplicación de
  referencia y la lista de miembros del workspace gana o pierde una dependencia.** **Razón**: la capa
  de reexports (D-5) existe precisamente para que nadie más tenga que cambiar.

## Capacidades

### Capacidades nuevas

- `persistence-memory-adapter`: el contrato observable de que las implementaciones en memoria de los
  puertos de persistencia propiedad del dominio tienen exactamente un crate dueño; de que cada ruta
  que hoy resuelve una de ellas sigue resolviendo al mismo ítem; de que ningún puerto gana, pierde
  ni cambia una implementación; y de que la clasificación de durabilidad no cambia.

### Capacidades modificadas

- `persistence-api-surface`: dos enunciados de la spec entregada están redactados como absolutos
  permanentes, pero describen la frontera propia de CORE-PERSIST-A, y las ediciones legítimas de
  este cambio se leerían como violaciones de ellos. Deben acotarse a CORE-PERSIST-A: el requisito
  "No Consumer Outside The Two Crates Is Edited" (`spec.md:96-104`) y el punto de No-Goals "No
  implementation move — every `InMemory*` and `PostgreSQL*`/`Postgres*` adapter stays in its current
  crate" (`spec.md:131-132`). **Ningún requisito sobre forma de puerto, resolución de rutas o
  identidad de traits cambia.**
- `foundation-integrity`: **no se espera modificación.** D-2 no requiere cambio de matriz:
  `foundation → domain` e `infrastructure → foundation` ya están permitidos
  (`xtask/src/layers.rs:77,80-86`). La entrada en `layers.toml` satisface el requisito de completitud
  existente (FR-001) en lugar de modificarlo. Se lista aquí solo para que la fase de spec lo confirme
  en vez de asumirlo.

Si la fase de spec encuentra que un requisito existente ya implica alguno de estos, lo integra en
lugar de fabricar un delta.

## Enfoque

Crear el crate; mover cada archivo de implementación con su cuerpo sin editar y sus líneas `use`
reapuntadas; reemplazar cada declaración vaciada por un `pub use ego_persistence_memory::…` en la
ruta idéntica; actualizar los imports de la aplicación de referencia; añadir la entrada de
`layers.toml` y el miembro del workspace. Nada más se edita.

El orden importa para la revisabilidad: cerrar el cierre de imports (D-3) antes de mover ningún
archivo; luego mover las cuatro implementaciones de `ego-infrastructure` (las más grandes, las mejor
cubiertas, protegidas por un único punto de reexport); luego el store de reservas con su aprobación
(D-7); luego los dos stores de read-side fuera del ejemplo. Cada paso deja el workspace compilando
con la capa de reexports intacta.

## Requisitos de aceptación

Cada uno es verificable de forma independiente y funciona también como criterio de éxito.

- [ ] **R1 — Propiedad canónica.** Cada una de las siete implementaciones resuelve desde exactamente
      un crate declarante, `ego-persistence-memory`, y no se declara en ningún otro sitio.
- [ ] **R2 — No se introduce ninguna implementación canónica duplicada.** El movimiento crea cero
      declaraciones nuevas; el número de bloques `impl <Puerto> for` por puerto movido es idéntico en
      todo el workspace.
- [ ] **R3 — Los dobles de prueba nombrados no se promueven.** `FakeDurableOffsetStore` y
      `FakeDurableDedupStore` siguen declarados en `examples/reference-app`, idénticos byte a byte, y
      no aparecen en el nuevo crate.
- [ ] **R4 — Lo ausente sigue ausente de forma visible.** `ProjectionStateStore` tiene cero
      implementaciones tras el cambio, y no se añade ningún stub, marcador ni implementación con
      `todo!()`.
- [ ] **R5 — Preservación del comportamiento.** El cuerpo de cada tipo movido —incluidas la
      resolución de tenant, la estrategia de bloqueo, la aritmética de conflicto de versión y el
      manejo fail-closed de tenant vacío (`read_side_store.rs:113-115`)— es textualmente idéntico a
      su forma previa, salvo ruta de módulo y líneas `use`.
- [ ] **R6 — Preservación de durabilidad y de producción.** Ningún tipo movido declara
      `is_durable()`; `presence_alone_is_not_durability` y ambas pruebas
      `try_build_rejects_explicit_in_memory_*` pasan sin modificación, y siguen rechazando stores en
      memoria bajo `Profile::Production`.
- [ ] **R7 — Neutralidad de backend.** `ego-persistence-memory` no contiene ninguna referencia a
      ningún backend —ningún tipo, dependencia ni feature flag de `sqlx`, Postgres, Stoolap, HTTP o
      Kafka— y no ofrece ninguna superficie de selección de backend.
- [ ] **R8 — Consolidación del read-side.** `InMemoryOffsetStore` e `InMemoryDedupStore` se declaran
      en `ego-persistence-memory` y ya no en `examples/reference-app`; el ejemplo los consume como
      dependencia ordinaria.
- [ ] **R9 — Reexports de compatibilidad en toda ruta antigua.** Cada ruta de la COMPATIBILITY
      REEXPORT MATRIX de `explore.md` sigue resolviendo, sin edición, al mismo ítem, demostrado en
      tiempo de compilación sobre la lista completa y no por muestreo. Los cinco archivos consumidores
      confirmados compilan con código fuente idéntico byte a byte.
- [ ] **R10 — Propiedad única de implementación por puerto movido.** Para `EventStore`,
      `EventStoreUnitOfWork`, `Repository`, `Snapshot`, `ReadSideStore`, `OffsetStore`, `DedupStore` y
      `OperationReservationStore`, `ego-persistence-memory` es el único dueño en memoria de propósito
      general; las únicas otras declaraciones que sobreviven son los dos duplicados nombrados de
      `persistent-entity` (D-9) y los dobles de prueba declarados.
- [ ] **R11 — Integridad de dependencias.** El `Cargo.toml` de `ego-persistence-memory` nombra
      exactamente `ego-persistence-api` y `ego-domain` como dependencias de ruta del workspace y nada
      más; no nombra ninguna dependencia de `ego-application`, `ego-runtime`, `ego-infrastructure`,
      `ego-persistence`, `ego-testkit`, transporte ni ejemplos. `cargo run -p xtask -- verify-layers`
      pasa sin violaciones nuevas y sin editar la matriz.
- [ ] **R12 — Integridad del alcance de efectos.** `crates/runtime/` y `crates/effect-store/` quedan
      sin modificar; `InMemoryEffectStore` y sus tres puertos son idénticos byte a byte; la frontera
      D-9 de CORE-PERSIST-A permanece intacta.
- [ ] **R13 — Ningún refactor de Postgres.** Ningún archivo SQL, de migración, de esquema ni de
      `crates/persistence/` aparece en el diff.
- [ ] **R14 — Ninguna expansión del framework de conformidad.** No se añade, extiende ni generaliza
      ningún arnés de conformidad; `assert_event_store_conformance` y las pruebas de lease de reservas
      conservan su forma y su ubicación actuales.
- [ ] **R15 — Ningún rediseño de contrato o trait.** `crates/persistence-api/src/**` queda sin
      modificar en absoluto; ningún puerto cambia su conjunto de métodos, cotas, supertraits, cuerpos
      por defecto ni object-safety.
- [ ] **R16 — Ningún doble de prueba de ningún tipo se promueve.** `TestClock` permanece en
      `ego-testkit`, y ningún doble local de `#[cfg(test)]` o de `tests/` se traslada al nuevo crate.
- [ ] **R17 — Los dos duplicados de `persistent-entity` son deuda nombrada, no manejo silencioso.**
      Ambos quedan registrados como KD-5 y KD-6 con responsables de seguimiento nombrados (F-5, F-6), y
      ninguno se mueve, fusiona, corrige ni atiende parcialmente.
- [ ] **R18 — La frontera del effect store es deuda nombrada, no manejo silencioso.** El cambio futuro
      se nombra **CORE-PERSIST-E** con su prerrequisito declarado (reubicación de puertos primero), y
      nada de esa frontera se toca.

## Deuda conocida (arrastrada, no corregida)

- **KD-1** — `ProjectionStateStore` sigue muerto: cero implementaciones, cero consumidores.
  Arrastrado desde CORE-PERSIST-A (NG-10).
- **KD-4** — La cobertura de conformidad es asimétrica: `Repository`, `Snapshot`, `OffsetStore` y
  `DedupStore` no tienen arnés. Por eso KD-5 se encontró leyendo el código a mano y no por una prueba
  en rojo (NG-4).
- **KD-5 — El `InMemorySnapshotStore` de `crates/persistent-entity/src/persistence.rs:733` ignora
  `tenant_id` por completo.** Defecto confirmado de aislamiento de tenant: dos tenants colisionan en
  el mismo `aggregate_id` (`:734,746-765`). **No se corrige aquí** (NG-1) → F-5.
- **KD-6 — El `InMemoryEventStore` de `crates/persistent-entity/src/persistence.rs:571` es una
  bifurcación no consolidada** que lleva una capacidad aditiva `with_version_offset()`
  (`:600-611,719-727`) de la que depende una prueba. **No se fusiona aquí** (NG-2) → F-6.

## Seguimientos nombrados

- **F-1 — CORE-PERSIST-E**: reubicar `EffectStateStore`, `EffectDedupStore` y `RetentionMaintenance`
  fuera de `ego-runtime` y luego consolidar `InMemoryEffectStore` en `ego-persistence-memory`
  (D-10, NG-6).
- **F-5 — Corregir el `InMemorySnapshotStore` de `persistent-entity` que ignora el tenant** como
  corrección independiente y revisada, con pruebas propias y radio de impacto declarado (KD-5). No
  debería esperar a la serie CORE-PERSIST.
- **F-6 — Decidir el destino del `InMemoryEventStore` bifurcado de `persistent-entity`**: fusionar la
  capacidad `with_version_offset` en la implementación canónica, o conservar la bifurcación con una
  razón declarada (KD-6).
- **F-4 — CORE-PERSIST-D**: arneses de conformidad para `Repository`, `Snapshot`, `OffsetStore` y
  `DedupStore` (KD-4, NG-4). **CORE-PERSIST-C** es dueño de la consolidación de PostgreSQL (NG-3).

## Áreas afectadas

| Área | Impacto | Descripción |
|------|---------|-------------|
| `crates/persistence-memory/` | Nueva | El crate completo: `Cargo.toml` + siete implementaciones reubicadas (IS-1, IS-2) |
| `crates/infrastructure/src/persistence/in_memory/` | Modificada | Cuatro declaraciones sustituidas por reexports en rutas idénticas (IS-3) |
| `crates/testkit/src/reservation.rs`, `crates/testkit/src/lib.rs` | Modificada | Store reubicado; `TestClock` y pruebas colocalizadas se quedan; reexport añadido (D-8, IS-3) |
| `examples/reference-app/src/read_side/store.rs` | Modificada | Dos declaraciones eliminadas; imports reapuntados (IS-4, D-6) |
| `layers.toml`, `Cargo.toml` raíz | Modificada | Una entrada de capa, un miembro de workspace (IS-1) |
| `xtask/src/layers.rs` | Intacta | No se requiere cambio de matriz (D-2) |
| `crates/persistence-api/` | Intacta | Ningún cambio de contrato (NG-5, R15) |
| `crates/persistent-entity/`, `crates/runtime/`, `crates/effect-store/`, `crates/persistence/` | Intactas | Diferidas (NG-1, NG-2, NG-3, NG-6) |
| `openspec/specs/{persistence-memory-adapter,persistence-api-surface}/spec.md` | Nueva / Modificada | Deltas según IS-6 |

## Riesgos

| ID | Riesgo | Probabilidad | Mitigación |
|----|--------|--------------|------------|
| R-1 | **El cambio de alcanzabilidad de D-7 se aprueba por omisión**: el store de reservas pasa en silencio a ser cableable en producción sin que nadie decida que deba serlo | Media | D-7 lo declara como decisión autónoma con fundamento propio, y R11 fija el conjunto de dependencias resultante. Si un revisor lo rechaza, el resultado correcto es sacar el ítem 7 de IS-2, no debilitar D-7 |
| R-2 | **Un cuerpo movido se desvía**: un bloqueo, una clave de tenant o un término de versión alterados dentro de un diff demasiado ancho para leerlo línea a línea | Media | D-4 convierte lo literal en regla; R5 lo hace comprobable como comparación de texto y no como juicio |
| R-3 | **El cierre de imports es más amplio de lo que D-3 previó** y apply descubre una tercera arista necesaria a mitad del movimiento | Media | `Clock` ya se encontró así y quedó nombrado (D-3). `design.md` fija la lista exacta de imports contra el código antes de mover ningún archivo. Una tercera arista es una decisión de diseño, no una improvisación en apply |
| R-4 | **Presupuesto de revisión.** Siete implementaciones repartidas en tres crates vaciados superarán el presupuesto de 400 líneas | Alta | Previsto, no oculto. `sdd-tasks` debe partirlo por crate de origen (infrastructure → testkit → reference-app), manteniendo en cada rebanada la capa de reexports intacta para que todo estado intermedio compile en todo el workspace |
| R-5 | **La redacción acotada al cambio de `persistence-api-surface` se lee como prohibición permanente**, y este cambio parece una violación de spec | Media | Nombrado en Capacidades → Modificadas. La corrección consiste en acotar dos enunciados a CORE-PERSIST-A; ningún requisito sobre forma de puerto o resolución de rutas cambia. Si la fase de spec discrepa, esto se convierte en una pregunta bloqueante, no en una reinterpretación silenciosa |
| R-6 | **El comentario de cabecera de `layers.toml` ya está desactualizado**: la línea 6 dice `domain → nothing`, pero `xtask/src/layers.rs:76` concede la arista propia de dominio desde la D-4 de CORE-PERSIST-A. Un lector podría juzgar D-2 con el comentario | Baja | Señalado, no corregido aquí (NG-7). La matriz ejecutable es la autoridad. Merece un seguimiento de una línea fuera de este cambio |
| R-7 | **Se omite un reexport para una ruta que ninguna prueba ejercita**, y se rompe un consumidor solo en su build | Media | IS-5 exige una prueba en tiempo de compilación sobre la COMPATIBILITY REEXPORT MATRIX completa, no comprobaciones puntuales; `cargo build --workspace` cubre a todos los consumidores del árbol |

## Plan de reversión

**Un solo commit de reversión, en cualquier momento, sin ruptura externa.**

Como toda ruta vaciada queda reexportada (D-5), ningún crate fuera del nuevo crate y de sus dos
crates vaciados depende de la nueva distribución. Revertir consiste en: eliminar
`crates/persistence-memory/`, restaurar las declaraciones de `ego-infrastructure` y `ego-testkit`
desde el árbol previo, restaurar las dos declaraciones y los imports de la aplicación de referencia,
y quitar la entrada de `layers.toml` y el miembro del workspace. Nada más se toca en ninguna
dirección, y `xtask/src/layers.rs` nunca cambió, así que no hay estado de compuerta que deshacer.

Esto se mantiene **a mitad de camino**, que es lo que hace segura la partición de R-4: un
CORE-PERSIST-B parcialmente entregado es un workspace donde algunas implementaciones en memoria
viven en un segundo crate y todos los consumidores siguen compilando sin cambios. La única excepción
es la aplicación de referencia (D-6), que no tiene capa de compatibilidad: su reversión es la
restauración de imports en dos archivos, contenida en un ejemplo hoja sin consumidores aguas abajo.

Ningún dato, esquema, migración ni estado persistido interviene en ninguna dirección. Este cambio no
escribe nada en tiempo de ejecución.

## Dependencias

- `persistence-api-surface` (entregada, CORE-PERSIST-A) — los puertos que este crate implementa, se
  consumen sin cambios salvo el reacotamiento documental nombrado en Capacidades.
- `foundation-integrity` (archivada) — FR-001 (completitud), FR-002 (dirección), FR-003 (sin ciclos),
  FR-005 (compilación aislada), consumidos sin cambios.
- Regla de diseño de `openspec/config.yaml` "No circular dependencies between crates", sostenida por
  construcción (D-2, D-3).
- La MOVE MATRIX y la COMPATIBILITY REEXPORT MATRIX de `explore.md`, con la corrección de D-3
  aplicada.
- Ningún crate externo, servicio ni infraestructura nueva.

## Ronda de preguntas de propuesta

Esta propuesta se produjo sin ronda interactiva. Cuatro preguntas de producto la afinarían; hasta
que se respondan, rige el supuesto declarado.

1. **Alcanzabilidad de D-7** — ¿hacer `InMemoryOperationReservationStore` cableable en producción es
   un resultado buscado, o debería permanecer inalcanzable desde código de producción hasta que
   exista una historia operativa para él? *Supuesto: es buscado — es la única implementación del
   puerto y reclama fidelidad de producción en su propio comentario.*
2. **¿Quién es el cliente del nuevo crate?** ¿Adoptantes del framework que escriben pruebas contra
   puertos reales, o también despliegues pequeños que genuinamente quieren persistencia en memoria?
   *Supuesto: adoptantes y pruebas; el uso en producción sigue bloqueado por `Profile::Production`
   (D-11).*
3. **Urgencia de KD-5** — el defecto de aislamiento de tenant es real y está entregado. ¿Debe
   planificarse F-5 antes del resto de la serie CORE-PERSIST en lugar de después? *Supuesto: es
   independiente y no debería esperar, pero esta propuesta no lo planifica.*
4. **Política de ejemplos de D-6** — ¿debe prohibirse a `examples/reference-app` declarar
   implementaciones genéricas de adaptador en adelante, como regla permanente, o esto es una limpieza
   puntual? *Supuesto: limpieza puntual; aquí no se propone ninguna regla permanente.*
