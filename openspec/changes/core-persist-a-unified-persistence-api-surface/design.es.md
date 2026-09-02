# Diseño: CORE-PERSIST-A — Superficie Unificada de API de Persistencia (Puertos Propiedad del Dominio)

> Compañero de revisión en español. Fuente de verdad canónica: `design.md` (identificadores 1:1).
>
> **Entradas**: `proposal.md` (D-1 … D-9, OD-1, IS-1 … IS-7, OOS-1 … OOS-14, KD-1 … KD-4,
> R-1 … R-6, F-1 … F-4, SC-1 … SC-12) y `explore.md` (§1, §3, §5, §8, §9, §11). Este
> documento decide el **cómo**: dirección de dependencia, contenido del crate, granularidad de
> las reexportaciones, relajación de la compuerta y fronteras de rebanada. Los requisitos
> observables son de `spec.md` y no se repiten aquí.
>
> **Lectura base**: `develop` @ `885d1da`. Cada archivo:línea de abajo se leyó sobre esta base,
> no se recordó de las entradas.

## Enfoque Técnico

Un crate hoja nuevo, una arista unidireccional, una relajación de compuerta y una capa de
reexportación con granularidad de **módulo**, de modo que ninguna sentencia `use` cambie en
ninguna parte — ni siquiera dentro del propio `ego-domain`.

`ego-persistence-api` no depende de **ningún crate del workspace**. `ego-domain` depende de él
y republica cada módulo reubicado en su ruta anterior exacta. Esa dirección la fuerza el código
que se queda (`read_side/scheduler.rs:5-10`, `session.rs:5-13`, `runner.rs:3-10` consumen
`DedupStore` / `OffsetStore` / `ReadSideStore`), y la arista inversa es imposible una vez que se
sostiene: Cargo rechaza el ciclo antes de que `foundation-integrity` llegue a ejecutarse.

**No se incluye diagrama de secuencia, y es una decisión deliberada de aplicabilidad.**
La regla de diseño de `openspec/config.yaml` pide uno para flujos asíncronos complejos. Este
cambio agrega, elimina y reordena cero rutas de llamada: cada elemento `#[async_trait]` se mueve
con una firma idéntica byte a byte (OOS-4), así que cualquier diagrama trazado aquí retrataría un
flujo que el cambio no toca. La estructura crítica es el **grafo de dependencias**, más abajo.

---

## Correcciones de Evidencia

Cuatro, todas halladas leyendo la base y no las entradas. Cada una cambia lo que la
implementación debe hacer.

### EC-1 — La clausura de tipos compartidos son **cinco** tipos, no los dos que nombra D-3

D-3 afirma que `ego-persistence-api` debe alcanzar `DomainEvent` y `TenantId`. Un grep de
`^use crate::|^use super::` sobre los diez archivos que IS-2 reubica encuentra tres más, todos en
`read_side/` pero fuera de la lista de archivos de IS-2:

| Tipo que necesita | Lo necesita | Definido en |
|---|---|---|
| `DomainEvent` | `persistence/event_store.rs:3` | `event.rs:47` |
| `TenantId` | `operation/receipt.rs:11`, `operation/reservation.rs:32` | `context.rs:56` |
| **`EventTag`** | `read_side/offset.rs:6`, `dedup.rs:6`, `store.rs:7`, `projection_state_store.rs:8` | `read_side/event_tag.rs:12` |
| **`ProjectionState`** | `read_side/projection_state_store.rs:9` | `read_side/state.rs:16` |
| **`EventStreamElement`** | `read_side/store.rs:6` | `read_side/event_stream.rs:13` |

Bajo la arista unidireccional de AD-1, cada uno de estos debe estar dentro de
`ego-persistence-api`. OD-1 se coteó contra dos tipos; debe recotearse contra cinco.
**AD-2 lo resuelve.**

### EC-2 — `TenantId` se genera por macro, así que "reubicar `TenantId`" no es mover un archivo

`context.rs:7` define `macro_rules! id_type!`; la línea 56 genera `TenantId` / `TenantIdError` a
partir de él, y la línea 55 y sus hermanas generan `EntityId`, `AggregateId`, `CorrelationId`,
`CausationId` y `RequestId` con el mismo generador. No existe un archivo `TenantId` que mover.
D-6 exige reubicación textual, y expandir la macro a mano en el destino no es textual.
**AD-3 lo resuelve.**

### EC-3 — El orden de PRs encadenados de R-4 está invertido; `persistence/` depende de `operation/`

R-4 recomienda rebanar `persistence/` → `read_side/` → `operation/`. El código fuente dice que
la primera flecha apunta al revés:

- `persistence/event_store.rs:4` → `crate::operation::OperationReceipt`
- `persistence/stored_event.rs:1` → `crate::operation::OperationKey`
- `operation/key.rs` tiene **cero** importaciones `crate::` / `super::` — es el piso de la clausura.
- Los cuatro archivos reubicados de `read_side/` no referencian nada de `persistence/` ni `operation/`.

El orden correcto es `read_side/` → `operation/` → `persistence/`. **AD-6 dispone las rebanadas.**

### EC-4 — El conteo autoritativo de elementos es **35**, no 27

27 es el conteo de las filas propiedad del dominio del §9 de la exploración. D-7 nombra 7
elementos que el §9 omite (→ 34). La enumeración directa de
`^pub (trait|struct|enum|fn|type|const)` sobre los diez archivos reubicados da **35**:
`operation/key.rs:19` declara `pub const MAX_LEN: usize = 255`, público en
`ego_domain::operation::key::MAX_LEN` y no nombrado ni por el §9 ni por D-7. Según D-7 manda la
regla —"todo elemento público de un módulo reubicado"— y 35 es el número que debe cubrir la
prueba de compilación de IS-6.

---

## Grafo de Dependencias

**Antes** — `ego-domain` es una hoja sin dependencias internas (`crates/domain/Cargo.toml:6-17`
nombra solo crates externos):

```
                    ego-domain  [domain]   ← hoja
                        ▲
   ┌────────┬───────────┼───────────┬──────────┬────────────┐
ego-application  ego-persistence  ego-runtime  persistent-entity  …
```

**Después** — una hoja nueva debajo, una arista nueva:

```
              ego-persistence-api  [domain]   ← hoja: sin dependencia del workspace
                        ▲
                        │  la única arista nueva, en una sola dirección
                    ego-domain     [domain]
                        ▲
   ┌────────┬───────────┼───────────┬──────────┬────────────┐
ego-application  ego-persistence  ego-runtime  persistent-entity  …
                (todos sin cambios — protegidos por la capa de reexportación, D-5)
```

**No se introduce ningún ciclo, y es verificable en lugar de afirmado.**
`crates/persistence-api/Cargo.toml` no nombra ninguna dependencia `path =`, así que la arista
inversa no existe como hecho sobre el archivo, no como promesa de revisión. Si alguien la
agregara, Cargo se niega a resolver el workspace antes de que `xtask verify-layers` corra, y la
verificación de ciclos de FR-003 se niega otra vez. La relajación de compuerta de AD-1 ensancha
la *matriz de capas*, nunca el *grafo de crates*.

---

## Decisiones de Arquitectura

### AD-1 — Dirección: `ego-domain → ego-persistence-api`; `allowed_layers("domain")` se relaja a `Some(&["domain"])`

**Decisión** — `xtask/src/layers.rs:76`:

```rust
"domain" => Some(&["domain"]),   // antes: Some(&[])
```

más `layers.toml`: `"ego-persistence-api" = "domain"`, y una línea en
`crates/domain/Cargo.toml`.

**Criterios**:

1. **La arista es forzada, no elegida.** `read_side/scheduler.rs:5-10` importa `DedupStore`,
   `OffsetStore` y `ReadSideStore`; `session.rs:5-13` y `runner.rs:3-10` hacen lo mismo. Esos
   tres archivos se quedan en `ego-domain` y consumen puertos que salen de él. La arista existe
   con independencia de la comodidad de reexportación de D-5 — eliminar el requisito de
   reexportación no la eliminaría.
2. **La relajación es la más estrecha disponible.** `Some(&["domain"])` es una auto-arista de
   misma capa, exactamente la forma que la matriz ya concede a `foundation`
   (`Some(&["domain","foundation"])`, `layers.rs:77`) y a `infrastructure` (auto-referencial,
   `layers.rs:80-86`). Admite `domain → domain` y nada más: `domain → foundation`,
   `domain → infrastructure` y `domain → sdk` siguen fallando en `check_direction`
   (`layers.rs:107-116`), que es lo que afirma SC-7.
3. **No es la regla "sin dependencias circulares entre crates" siendo doblada.** Esa regla es
   sobre el grafo de crates, que sigue siendo un DAG (ver Grafo de Dependencias). La matriz de
   capas es una regla de *dirección* sobre capas, y una arista de misma capa no es una violación
   de dirección — es el caso que la matriz antes no tenía forma de expresar, porque `ego-domain`
   era el único crate `domain` del workspace. Mapear el crate nuevo a `foundation` sería un
   agujero mucho más ancho: legalizaría `ego-domain → ego-runtime`.

**Consideradas y rechazadas** (ambas son rutas de OD-1, nombradas aquí para el registro y
diseñadas en ninguna parte de este documento):

- *Un tercer crate hoja con los tipos de valor compartidos, del que dependan ambos* — de forma
  correcta, pero agrega un segundo crate nuevo a una rebanada ya por encima de su presupuesto de
  revisión (R-4).
- *Redimensionar A1 para dejar `EventStore` / `Repository` atrás* — parte el vocabulario de
  puertos entre dos crates, que es exactamente la condición que este cambio existe para terminar.
- *Relajar la cota `DomainEvent` de `EventStore<E>`* — es un cambio de firma, prohibido por OOS-4.

### AD-2 — Los cinco tipos de EC-1 se reubican con los puertos; `ego-persistence-api` queda cerrado bajo compilación

**Decisión**: `read_side/event_tag.rs`, `read_side/state.rs`, `read_side/event_stream.rs` y
`event.rs` se reubican textualmente junto a los puertos, reexportados en sus rutas antiguas como
todo lo demás. `TenantId` es AD-3.

**Criterios**:

1. **Tres de los cinco ya son vocabulario de puertos del lado de lectura.** `OffsetStore`,
   `DedupStore`, `ReadSideStore` y `ProjectionStateStore` están *indexados* por `EventTag`;
   `ReadSideStore` *entrega* `EventStreamElement`. Su hogar ya era el equivocado — este diseño no
   amplía el alcance para alcanzarlos, sino que termina la frase que IS-2 empezó.
2. **Los tres son hojas.** `event_tag.rs` y `state.rs` tienen cero importaciones
   `crate::`/`super::`; `event_stream.rs:6` importa solo `EventTag`. Reubicarlos no arrastra nada
   más.
3. **`DomainEvent` (`event.rs`, 62 líneas, solo `chrono` + `serde_json`) es el único elemento
   cuyo hogar este diseño empeora, y lo dice.** Es el contrato central de eventos del dominio
   (`lib.rs:109`), no un puerto de persistencia. Se reubica solo porque
   `EventStore<E: DomainEvent>` no puede compilar sin él y las alternativas son las tres
   rechazadas en AD-1.

**Costo, declarado en lugar de enterrado**: esto excede la lista de archivos de IS-2 y contradice
el "reubica ese vocabulario —**y nada más**" del Objetivo. No viola ningún OOS (sin cambios de
firma, sin tipos nuevos, ninguna implementación se mueve), pero la frase de alcance de la
propuesta queda ahora inexacta. **OQ-1 rastrea la enmienda requerida.**

### AD-3 — `id_type!` se reubica y se `#[macro_export]`a; `ego-domain` lo invoca para sus cuatro tipos de identidad restantes

**Decisión**: el bloque `macro_rules! id_type` (`context.rs:7-54`) se mueve a
`ego-persistence-api` y gana `#[macro_export]`. `TenantId` / `TenantIdError` se generan allí. El
`context.rs` de `ego-domain` sigue generando `AggregateId`, `EntityId`, `CorrelationId`,
`CausationId` y `RequestId` invocando la macro reexportada, y reexporta `TenantId` /
`TenantIdError` en `ego_domain::context::TenantId` y `ego_domain::TenantId` (`lib.rs:103-107`).

**Criterios**: (a) una sola definición del generador, no dos — la alternativa (copiar la macro al
crate nuevo) deja un generador de 47 líneas duplicado, que es la clase de deriva que D-6 existe
para prevenir; (b) la macro se mueve textualmente, satisfaciendo D-6, y el tipo que genera es un
solo tipo, satisfaciendo el "no una copia redeclarada que solo comparte el nombre" de SC-1;
(c) expandir la macro a mano solo para `TenantId` se rechazó de plano — es la única ruta aquí
que *no* es textual.

**Es la decisión más incómoda del documento**: un generador de identidad de dominio termina en un
crate de persistencia. Queda registrada como **OQ-2**, no suavizada.

### AD-4 — Las reexportaciones se declaran con granularidad de **módulo**, lo que reduce el recableado interno a cero

**Decisión**: cada declaración de módulo vaciada de `ego-domain` se vuelve una reexportación de
módulo, y las líneas `pub use` a nivel de elemento se dejan idénticas byte a byte:

```rust
// crates/domain/src/persistence/mod.rs — después
pub use ego_persistence_api::persistence::{
    error, event_store, repository, snapshot, stored_event, tenant,
};
pub use error::PersistenceError;                       // sin cambios, resuelve a través de lo anterior
pub use event_store::{EventStore, EventStoreUnitOfWork};
pub use repository::Repository;
pub use snapshot::Snapshot;
pub use stored_event::StoredEvent;
pub use tenant::resolve_tenant;
```

**Criterios**: (a) `super::event_tag::EventTag` (`handler.rs`, `processor.rs`, `progress.rs`,
`tagger.rs`, `session.rs`, `runner.rs`) y `crate::read_side::dedup::DedupStore`
(`scheduler.rs:5`) resuelven a través de un módulo reexportado sin edición — **el "recablear los
consumidores internos de `ego-domain`" de IS-4 se reduce a nada**, y el propio crate del cambio
queda tan intacto como todo consumidor fuera de él; (b) la reexportación a nivel de elemento
obligaría a cambiar cada una de esas líneas `use`, agregando ruido a un diff que R-4 ya marca
como demasiado grande para leer línea a línea; (c) los seis nombres explícitos de módulo le ganan
a `pub use …::*` porque un glob vuelve invisible un módulo faltante hasta que falla la
compilación de alguien aguas abajo (R-6).

### AD-5 — El `Cargo.toml` de `ego-persistence-api` se deriva, no se adivina

**Decisión**: partir del bloque `[dependencies]` de `ego-domain` (`Cargo.toml:6-17`) y luego
borrar toda entrada que el conjunto reubicado no nombre, probado con
`cargo build -p ego-persistence-api` en aislamiento (FR-005). Se sabe que `sha2` se mueve —
`crates/domain/Cargo.toml:13-17` lo documenta como existente únicamente para `OperationKeyHash`
(`operation/key.rs:203`), que se reubica—, así que se espera que `ego-domain` lo pierda. Las
`[dev-dependencies]` `mockall` y `tokio` siguen a los módulos `#[cfg(test)]` que se mueven con sus
archivos (D-6).

**Criterios**: el conjunto de dependencias es un hecho de compilación, y listarlo de memoria aquí
sería el único lugar de una reubicación textual donde este documento inventaría algo.

### AD-6 — Tres rebanadas, ordenadas por la clausura, cada una compilando en todo el workspace por sí sola

`sdd-tasks` es dueño de la descomposición en tareas; este diseño es dueño solo de las fronteras
que EC-3 vuelve obligatorias.

| Rebanada | Contenido | Clausura que necesita | Lista |
|---|---|---|---|
| **S1 — lado de lectura** | esqueleto del crate, `Cargo.toml`, entrada en `layers.toml`, **la relajación de compuerta de AD-1 + su prueba**, `read_side/{offset,dedup,store,projection_state}` + `event_tag`, `state`, `event_stream` (AD-2), reexportaciones de módulo | ninguna — todo archivo es hoja o depende solo de S1 | **ahora** |
| **S2 — operación** | `operation/{key,receipt,reservation}`, `id_type!` + `TenantId` (AD-3), reexportaciones de módulo | `TenantId` (AD-3) | tras OQ-2 |
| **S3 — persistencia** | `persistence/{error,event_store,repository,snapshot,stored_event,tenant}`, `event.rs` (AD-2), reexportaciones de módulo | `OperationKey`/`OperationReceipt` de S2, más `DomainEvent` | tras S2 + OQ-1 |

**Criterios**: (a) S1 carga la relajación de compuerta porque introduce la primera arista — la
relajación aterriza *junto con* la arista que la necesita, nunca antes; (b) S1 no está bloqueada
por ninguna de las dos preguntas abiertas, así que el trabajo puede empezar mientras se deciden;
(c) cada rebanada mantiene intacta la capa de reexportación, de modo que un CORE-PERSIST-A
aterrizado parcialmente es un workspace donde algunos puertos viven en un segundo crate y todo
consumidor sigue compilando sin cambios (la propiedad de reversión en pleno vuelo de la
propuesta).

### AD-7 — `ProjectionStateStore` se reubica muerto, y el defecto de `PostgreSQLRepository` no se toca

**Decisión**: `ProjectionStateStore` + `ProjectionStateStoreError` se mueven textualmente con
cero implementaciones y cero consumidores (KD-1, D-8) — eliminarlos convertiría una reorganización
en un cambio de comportamiento. `crates/persistence/src/postgres/repository.rs` **no se abre**:
su alcance `tenant_id = $2` (líneas 82, 135, 161) y su `ON CONFLICT (aggregate_id, tenant_id)` de
la línea 109, que apunta a una restricción que `002_create_aggregates.sql` nunca declara —un
`42P10` vivo de Postgres y un defecto de aislamiento de tenant— quedan exactamente como están
(KD-2, OOS-12, propiedad de F-2). KD-3 y KD-4 igualmente se arrastran sin tocar.

**Criterios**: este cambio reubica puertos; ese archivo contiene una implementación (OOS-1) y su
corrección necesita sus propias pruebas y su propio calendario. F-2 explícitamente *no* está
supeditada a la serie CORE-PERSIST.

---

## Puntos de Integración

| Frontera | Dirección | Mecanismo | Verificado en |
|---|---|---|---|
| `ego-domain` → `ego-persistence-api` | nueva, una vía | dependencia `path` + reexportaciones de módulo | `crates/domain/Cargo.toml`; AD-1, AD-4 |
| `ego-persistence-api` → cualquier crate del workspace | **ninguna** | no existe dependencia `path` | `crates/persistence-api/Cargo.toml`; AD-5 |
| todo consumidor existente → elementos movidos | sin cambios | resuelto por las reexportaciones de `ego-domain` | 92 archivos, §6 de la exploración; SC-2 |
| consumidores internos de `ego-domain` → puertos movidos | sin cambios | las rutas `super::`/`crate::` resuelven por módulos reexportados | AD-4 |
| `layers.toml` → `verify-layers` | entrada | una entrada nueva, cargador existente | `layers.rs:150-158` |
| `allowed_layers` → `check_direction` | entrada | un brazo de `match` | `layers.rs:76`, `:107-116`; SC-7 |
| comportamiento en ejecución | **ninguno** | nada se ejecuta distinto | OOS-6 |

Cero plomería nueva: un crate, una arista, un brazo de `match`.

## Estrategia de Pruebas

TDD estricto. La prueba RED es el archivo de identidad de reexportación — nombra rutas
`ego_persistence_api::` que aún no existen, así que falla al compilar antes de cualquier
reubicación.

| Nivel | Ubicación | Qué prueba |
|---|---|---|
| Tiempo de compilación (principal) | `crates/persistence-api/tests/reexport_identity.rs` | **SC-1 / IS-6** sobre los **35** elementos (EC-4). Un `use` pelado no basta: compila contra una copia redeclarada. Cada elemento recibe un testigo de identidad: una coerción de identidad para traits object-safe (`fn f(x: Box<dyn ego_domain::…::DedupStore>) -> Box<dyn ego_persistence_api::…::DedupStore> { x }`), y para traits genéricos un testigo con cláusula `where` que carga ambas cotas sobre un mismo parámetro. Lista completa, nunca una muestra |
| Unitario | `xtask/src/layers.rs` `#[cfg(test)]` | **SC-7**: `domain → domain` no arroja violación, y `domain → foundation` / `domain → infrastructure` / `domain → sdk` siguen arrojando `WrongDirection`. Sigue la forma existente de `graph_from` / `layers_from` (`layers.rs:164-208`) |
| Reubicado | los módulos `#[cfg(test)]` movidos | **SC-3 / D-6**: se mueven textualmente con sus archivos. El conteo de aserciones antes y después debe ser idéntico — una aserción modificada es señal de deriva semántica, no una limpieza |
| Compuerta | `cargo run -p xtask -- verify-layers` | **SC-6**: mapeado (FR-001), arista permitida (FR-002), sin ciclo (FR-003), compilación en aislamiento (FR-005) |
| Workspace | `cargo build --workspace`, `cargo test --workspace` | **SC-5**: cero fallos nuevos, cero aserciones modificadas |

Tres propiedades son **propiedades del diff**, verificadas leyendo el cambio y no con una prueba:
**SC-2** (ninguna edición de `use` ni de `Cargo.toml` fuera de los dos crates), **SC-8** (ningún
archivo `.sql`/de migración en el diff) y **SC-9** (`crates/runtime/`, `crates/effect-store/` y
toda implementación de OOS-1 ausentes de la lista de archivos).

## Matriz de Amenazas

N/A — sin frontera de enrutamiento, comando de shell, subproceso, automatización de VCS/PR,
clasificación de archivos ejecutables ni integración de procesos. Este cambio mueve archivos
fuente de Rust entre dos crates y agrega un brazo de `match`.

`ego-rs-security` aplica solo para confirmar que queda intacto: **cero** texto SQL, construcción
de consultas, ruta de autenticación, verificación de JWT o comprobación de `CrossTenantPermit`
aparece en el diff (OOS-3). La regla de tres vías de `resolve_tenant` (`persistence/tenant.rs:29`)
se reubica textualmente bajo OOS-5, así que la semántica de tenant es idéntica byte a byte antes
y después. Reglas 1 a 4: **PASA por ausencia**, no por argumento.

## Migración / Despliegue

**No se requiere migración.** No existe dato, esquema, archivo de migración ni estado persistido
en ninguna dirección — este cambio no escribe nada en tiempo de ejecución (OOS-6). Sin bandera de
funcionalidad y sin despliegue por fases: el workspace compila con la disposición nueva o no.

La reversión es la de la propuesta, sin cambios y disponible en pleno vuelo según AD-6: eliminar
`crates/persistence-api/`, restaurar los módulos de `ego-domain` desde el árbol previo, quitar la
única arista de `Cargo.toml` y la entrada de `layers.toml`, y restaurar `allowed_layers("domain")`
a `Some(&[])`.

## Trazabilidad

| Elemento de la propuesta | Resuelto por | Nota |
|---|---|---|
| **OD-1**, D-2, D-4, IS-5 | **AD-1** | dirección forzada por `scheduler.rs:5-10`; `Some(&[])` → `Some(&["domain"])`; ambas alternativas nombradas y rechazadas |
| D-3 | **EC-1 + AD-2 + AD-3** | la clausura son cinco tipos, no dos; `TenantId` se genera por macro |
| D-1, IS-1 | AD-5 | crate en `crates/persistence-api/`, mapeado a `domain`, conjunto de dependencias derivado compilando |
| D-5, IS-3, R-6 | **AD-4** | reexportación con granularidad de módulo, nombres explícitos, sin glob |
| D-6, R-2, SC-3, SC-4 | AD-2, AD-3, Pruebas | textual; las impls de reenvío `Arc<T>` (`offset.rs:92`, `dedup.rs:60`) se mueven dentro de sus propios archivos |
| D-7, R-5, SC-1 | **EC-4** | 35 elementos, no 27 — `MAX_LEN` (`key.rs:19`) no lo nombra ni el §9 ni D-7 |
| D-8, KD-1, OOS-9 | AD-7 | `ProjectionStateStore` se reubica muerto |
| D-9, OOS-2, SC-9, F-1 | — | `ego-runtime` / `ego-effect-store` no aparecen en ninguna lista de archivos aquí |
| IS-4 | **AD-4** | se reduce a cero ediciones — la reexportación de módulo mantiene resolviendo las rutas `super::`/`crate::` |
| IS-6, SC-1 | Pruebas | testigo de identidad por elemento, no un `use` pelado, no una muestra |
| R-3, SC-7 | AD-1 criterio 2 + Pruebas | la matriz admite `domain → domain` y nada más ancho, afirmado |
| **R-4, SC-12** | **EC-3 + AD-6** | el orden de R-4 está invertido; el correcto es `read_side/` → `operation/` → `persistence/` |
| R-1 | **OQ-1, OQ-2** | la mitad de dirección de OD-1 queda cerrada; su mitad de clausura se recotea, no se asume resuelta |
| KD-2, OOS-12, F-2 | AD-7 | el `42P10` y el defecto de alcance de tenant se arrastran sin tocar |
| KD-3, KD-4, OOS-13, OOS-14, F-3, F-4 | AD-7 | arrastrados, no corregidos |
| `config.yaml` "sin dependencias circulares" | Grafo de Dependencias | arista de una vía; `crates/persistence-api/Cargo.toml` no nombra ningún crate del workspace |
| `config.yaml` "diagramas de secuencia" | Enfoque Técnico | N/A explícito — sin cambios de flujo asíncrono |

## Preguntas Abiertas

- [ ] **OQ-1 — AD-2 excede IS-2 y contradice el "y nada más" del Objetivo.**
      Reubicar `DomainEvent`, `EventTag`, `ProjectionState` y `EventStreamElement` lo fuerza EC-1
      bajo la arista de una vía de AD-1, y no viola ningún OOS — pero la lista de archivos de IS-2
      y la frase del Objetivo quedan ahora inexactas. **Confirmar la enmienda a la propuesta antes
      de que `sdd-tasks` planifique S3.** S1 no se ve afectada y puede empezar.
- [ ] **OQ-2 — AD-3 pone `id_type!`, un generador de identidad de dominio, en
      `ego-persistence-api`.** Es la única ruta que es a la vez textual (D-6) y de definición
      única. La alternativa es duplicar una macro de 47 líneas. Confirmar antes de que
      `sdd-tasks` planifique S2.
