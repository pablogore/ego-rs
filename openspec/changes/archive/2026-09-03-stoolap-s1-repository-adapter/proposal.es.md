# Propuesta: STOOLAP-S1 — Adaptador `Repository` de Stoolap de Primera Clase

> Compañero de revisión en español. Fuente canónica: `proposal.md` (identificadores 1:1:
> D-1..D-12, NG-1..NG-9, IS-1..IS-6, R1..R14, KD-1..KD-3, F-1..F-4, RK-1..RK-7).
>
> Base factual: la exploración de STOOLAP-S1 (Engram `sdd/stoolap-s1/explore`, veredicto **GREEN**).
> Sus cinco hallazgos de capacidad sobre stoolap 0.4.0 se consumen tal cual, no se vuelven a derivar.

## Objetivo

Añadir **una sola** implementación de `ego_persistence_api::persistence::Repository<A>` respaldada
por Stoolap, en su propio crate, con **cero** dependencia del adaptador PostgreSQL, y un harness de
conformidad compartido entre backends que juzgue a Memory, PostgreSQL y Stoolap contra el mismo
contrato en lugar de contra la lectura que cada autor hizo de él.

## Intención

**La carencia es una tercera implementación ausente, no una abstracción ausente.** `Repository<A>`
tiene hoy exactamente dos implementaciones: `InMemoryRepository`
(`crates/persistence-memory/src/persistence/repository.rs:11`) y `PostgreSQLRepository`
(`crates/persistence/src/postgres/repository.rs:27`). Un despliegue que quiera persistencia durable
de agregados sin operar un servidor PostgreSQL no tiene opción, aun cuando este workspace ya publica
un proveedor Stoolap funcional y respaldado en disco para otro puerto
(`crates/effect-store/src/stoolap/`, PROD-002 Fase 4) y ya fija `stoolap 0.4.0` en `Cargo.lock`.

**La segunda carencia es de verificación, y a este workspace ya le costó una vez.**
`crates/testkit/src/event_store.rs:1-20` documenta por qué existe `assert_event_store_conformance`:
dos implementaciones de `EventStore` discrepaban en silencio sobre la partición systemwide (sin
tenant), ambas satisfacían la firma del trait, solo una satisfacía su significado, y *nada en el
workspace las comparaba*. `Repository` está hoy exactamente en ese estado previo al incidente: dos
implementaciones, ningún harness compartido (CORE-PERSIST-B lo registró como KD-4). Añadir una
tercera implementación sin harness triplica la superficie para la misma clase de divergencia.

**Dos hallazgos hacen que este adaptador no sea obvio, y ambos se deciden aquí en lugar de
descubrirse durante la implementación.** La verificación de índices únicos de Stoolap se *omite por
completo* cuando una columna indexada es NULL, y Stoolap no tiene índices parciales, de modo que el
truco de dos índices parciales que usa PostgreSQL para el tenant (migración 015) no tiene
equivalente y una traducción ingenua permitiría filas systemwide duplicadas en silencio. Además, el
modo de sincronización por defecto de Stoolap no hace fsync en cada commit, así que la durabilidad
es opt-in. Ambos puntos son D-5 y D-6 más abajo.

## Decisiones activas

| ID | Decisión | Fundamento / evidencia |
|----|----------|------------------------|
| D-1 | **Nuevo crate `ego-persistence-stoolap` en `crates/persistence-stoolap/`**, hermano de `ego-persistence` (PostgreSQL) y `ego-persistence-memory` (en memoria) | Respeta la convención paquete `ego-*` / directorio sin prefijo que esos dos ya fijaron (`crates/persistence-memory/Cargo.toml:2`, lista de miembros del workspace `Cargo.toml:5-8`). El nombre se confirmó contra el workspace, no se heredó de la descripción de la tarea |
| D-2 | **La clasificación de capa es `infrastructure`**, no `foundation`. `layers.toml` incorpora `"ego-persistence-stoolap" = "infrastructure"` y **`xtask/src/layers.rs` no se modifica en absoluto** | Un adaptador que gobierna un motor de almacenamiento externo es infraestructura por la misma lectura que ubica ahí a `ego-persistence` (`layers.toml:27`) y a `ego-effect-store` (`:35`). `ego-persistence-memory` es `foundation` (`:36`) precisamente porque no tiene backend; este crate sí lo tiene. `infrastructure → domain` ya está permitido (`layers.toml:10`) y `ego-persistence-api` es `domain` (`:17`), así que no hace falta relajar ninguna compuerta |
| D-3 | **El conjunto de dependencias es `ego-persistence-api` + `stoolap` + los crates técnicos estrictamente necesarios. `ego-domain` queda deliberadamente *fuera*** | `Repository`, `PersistenceError` y `resolve_tenant` se resuelven directamente desde `ego-persistence-api` (`persistence-api/src/persistence/{repository.rs:12,tenant.rs:29}`), y nada en una implementación de `Repository<A>` necesita un tipo de valor de dominio, a diferencia de `ego-persistence-memory`, que necesitó `Clock` para otro puerto (D-3 de CORE-PERSIST-B). `design.md` es dueño de la lista final de imports |
| D-4 | **Sin puente asíncrono.** A diferencia de `PostgreSQLRepository`, este adaptador no necesita `tokio::task::block_in_place` / `Handle::block_on` | `Repository` es un trait **síncrono** (`repository.rs:21-39`) y la API de Stoolap es síncrona. `PostgreSQLRepository::block_on` (`postgres/repository.rs:51-53`) existe solo para puentear el `sqlx` asíncrono hacia un trait síncrono; ese puente no tiene razón de ser aquí. `ego-effect-store` necesitó `spawn_blocking` por el motivo inverso: *sus* puertos son asíncronos. Se declara para que nadie copie el puente por reflejo |
| D-5 | **AD — El ámbito de tenant se almacena como columna centinela NOT NULL, nunca como `NULL` de SQL.** El ámbito systemwide se codifica como cadena vacía en la tabla propia del adaptador; un único índice `UNIQUE (tenant_id, aggregate_id)` simple impone entonces una fila por ámbito de manera uniforme | Lo fuerzan dos hechos de Stoolap: `check_unique_constraints()` retorna anticipadamente —omitiendo por completo la verificación de unicidad— cuando cualquier columna indexada es NULL, y `CREATE INDEX` no admite predicado `WHERE` en absoluto. Una columna de tenant anulable permitiría entonces que existieran en silencio *filas systemwide duplicadas* para un mismo `aggregate_id`, y el desdoblamiento en dos índices parciales de PostgreSQL (`postgres/repository.rs:114-148`) no puede replicarse. `""` es seguro como centinela porque `resolve_tenant` rechaza `Some("")` como `MissingTenant` **antes de alcanzar cualquier adaptador** (`tenant.rs:32`), de modo que nunca puede colisionar con un tenant real. **Es únicamente una codificación de almacenamiento interna al adaptador**: el contrato externo `Option<&str>` de `Repository` queda intacto y ningún llamador ve jamás el centinela (R4). La mecánica completa corresponde a `design.md` |
| D-6 | **AD — El commit durable es una decisión explícita del adaptador, no un valor por defecto heredado.** El adaptador abre su base de datos solicitando durabilidad de sincronización completa en vez de aceptar el defecto de Stoolap | El `SyncMode::Normal` por defecto de Stoolap no hace fsync en cada commit; la durabilidad de commit equivalente a PostgreSQL requiere el parámetro de DSN `sync=full`. Esto se degrada de forma silenciosa e invisible; evidencia dentro del árbol: el proveedor Stoolap del effect-store abre `file://{path}` sin parámetro de sincronización (`crates/effect-store/src/stoolap/mod.rs:175`). El estado de un agregado no es una caché; un adaptador que pierde el último segundo de commits tras una caída no es un `Repository`. Se nombra como decisión para que un revisor apruebe la durabilidad deliberadamente y un test la fije (R5) |
| D-7 | **Todo conflicto de escritura se mapea a `PersistenceError::Conflict`. No se añade ninguna variante de error** | `PersistenceError` no tiene variante reintentables (a diferencia de `EffectStoreError::TemporarilyUnavailable`). Un `expected_version` obsoleto y una colisión de reclamo de escritura MVCC de Stoolap son el mismo evento desde la posición del llamador —*perdiste una carrera, recarga y reintenta*— y los llamadores ya reintentan ante `Conflict`. Inventar una variante reabriría el contrato publicado `persistence-api-surface` para servir a un solo backend (NG-5) |
| D-8 | **La concurrencia optimista se impone con una transacción real más una escritura condicional guardada por versión, no con bloqueo de fila** | Stoolap no tiene `SELECT ... FOR UPDATE` en ninguna parte. Sí tiene transacciones ACID reales con commit/rollback explícitos y detección MVCC de conflictos de reclamo de escritura por fila, que previene la misma escritura sucia contra la que protege el `FOR UPDATE` de `postgres/repository.rs:89-98`. El patrón refleja el enfoque CAS de Stoolap ya probado en `ego-effect-store`. La mecánica a nivel de sentencia corresponde a `design.md`, no a este documento |
| D-9 | **El harness de conformidad entre backends vive en `ego-testkit`**, junto a los harnesses que ya están ahí; no se copia por adaptador ni se ubica en ningún crate de adaptador | `crates/testkit/src/{event_store.rs,reservation_conformance.rs,observability_conformance.rs}` es el hogar establecido de este workspace para verificaciones de conformidad compartidas, y su propio comentario de documentación expone el motivo (`event_store.rs:1-20`). `ego-testkit` es capa `tooling`, un sumidero (`layers.toml:13,34`), así que una arista de dev-dependency desde cada crate adaptador es legal y no otorga alcance de producción. El `conformance.rs` interno de `ego-effect-store` es el patrón *anterior* y no se sigue aquí |
| D-10 | **La conformidad de PostgreSQL se ejecuta exclusivamente en el workspace separado `integration-tests/`; Memory y Stoolap se ejecutan en el workspace raíz** | `integration-tests/` es un workspace deliberadamente no miembro para que la raíz compile y pruebe **sin Docker** (`integration-tests/Cargo.toml:1-15`), y ya declara dev-dependency de `ego-testkit` (`:59`). Stoolap es embebido y respaldado en archivo, así que no necesita contenedor y pertenece a la suite raíz. **Bajo ninguna circunstancia se introduce una dependencia de Testcontainers en el workspace raíz** (R9) |
| D-11 | **Cero reutilización de PostgreSQL, enunciado como regla y no como aspiración.** El nuevo crate no debe nombrar `crate::postgres::*`, `PostgreSQLRepository`, `PgPool`, ningún helper de `sqlx`, ninguna migración de PostgreSQL, ninguna clasificación de errores de PostgreSQL ni ningún helper de test privado de PostgreSQL | Los dos backends comparten un *contrato*, no una implementación. Cualquier atajo a través del crate de PostgreSQL convertiría a `ego-persistence` en una clase base de facto y volvería a acoplar los backends que este cambio existe para mantener independientes. Verificable como aserción de dependencias y de símbolos (R7) |
| D-12 | **`Repository` es el único puerto implementado, y no se crea ninguna abstracción de backend** | Todo otro puerto queda sin implementación para Stoolap (NG-1), y no se introduce ningún `StorageEngine`, capa de dialecto SQL, ORM, motor genérico de repositorios ni toolkit de backends (NG-2). Un puerto, un backend, un adaptador honesto |

## Compuerta de atomicidad

**Ejecutar.** Un puerto, un backend, un crate, más el harness que prueba que las tres
implementaciones concuerdan. El harness no es separable: `crates/testkit/src/event_store.rs:1-20`
documenta el incidente exacto que ocurre cuando una segunda implementación de un puerto con ámbito
de tenant se publica sin uno, y la codificación centinela de D-5 hace que la partición por tenant de
este adaptador sea *estructuralmente distinta* de la de las dos implementaciones existentes, que es
precisamente la condición que un harness compartido existe para vigilar. Publicar el adaptador solo
dejaría la decisión de mayor riesgo de este cambio sin evidencia cruzada que la respalde.

Explícitamente **FUERA**, cada punto por ser una decisión independiente y no una pieza faltante de
esta: cualquier segundo store respaldado por Stoolap · cualquier capa de abstracción de backend ·
cualquier backend adicional · CORE-PERSIST-A2 · el renombrado `ego-persistence` →
`ego-persistence-postgres` · cualquier cambio a `ego-persistence-api`.

**ATOMICIDAD: PASS**, en línea con el veredicto GREEN de la exploración. Ningún contrato publicado
requiere modificación.

## Alcance

**Frontera de un vistazo**

| | |
|---|---|
| **STOOLAP-S1 incluye** | Nuevo crate `ego-persistence-stoolap` · `StoolapRepository<A, F>` implementando `Repository<A>` · esquema con centinela de tenant (D-5) · configuración de commit durable (D-6) · harness de conformidad de `Repository` compartido en `ego-testkit` · ese harness ejecutado contra Memory + Stoolap (raíz) y PostgreSQL (`integration-tests/`) · entrada en `layers.toml` + miembro del workspace |
| **STOOLAP-S1 excluye** | Todo otro store Stoolap · toda abstracción de backend · todo backend más allá de Memory/PostgreSQL/Stoolap · CORE-PERSIST-A2 · el renombrado del crate de persistencia · toda edición de `ego-persistence-api` · toda edición de PostgreSQL |

### Dentro del alcance

- **IS-1** — El nuevo crate (D-1), mapeado a `infrastructure` en `layers.toml` (D-2), añadido a la
  lista de miembros del workspace raíz, dependiendo solo del conjunto fijado por D-3.
- **IS-2** — `StoolapRepository<A, F>` implementando
  `ego_persistence_api::persistence::Repository<A>` —`save`, `load`, `delete`— con ámbito de tenant
  y concurrencia optimista, reflejando la forma pública de `PostgreSQLRepository<A, F>`
  (constructor que recibe un destino de conexión y un deserializador, `Debug`, los mismos límites
  genéricos) con cero dependencia de él (D-11).
- **IS-3** — El esquema propio del adaptador y su creación: la columna centinela de tenant y el
  único índice `UNIQUE (tenant_id, aggregate_id)` simple (D-5). Propiedad exclusiva de este crate;
  no se lee, comparte ni referencia ninguna migración de PostgreSQL.
- **IS-4** — Configuración de commit durable en la apertura (D-6), con un test que falla si el
  adaptador cae en el modo de sincronización por defecto de Stoolap.
- **IS-5** — Un harness de conformidad de `Repository` compartido en `ego-testkit` (D-9), que cubre
  como mínimo: avance de versión desde un agregado nuevo, rechazo de un `expected_version` obsoleto,
  fidelidad de ida y vuelta en la carga, `NotFound` en carga y borrado de un agregado ausente,
  `MissingTenant` ante `Some("")` y —con el rigor de
  `integration-tests/tests/infrastructure/repository_tenant_scoping_postgres.rs`— que el ámbito
  systemwide está aislado de todo tenant concreto y es igual a sí mismo entre llamadas.
- **IS-6** — Ese harness ejecutado contra las tres implementaciones según D-10, más los deltas de
  spec de la sección Capacidades.

### Fuera del alcance — No-objetivos

Cada punto es un **no-objetivo con motivo declarado**, no una omisión.

- **NG-1 — Ningún segundo store respaldado por Stoolap.** `EventStore`, `Snapshot`,
  `OperationReservationStore`, `OffsetStore` y `DedupStore` no reciben implementación Stoolap.
  **Motivo**: cada uno es un contrato distinto con su propia semántica de tenant, orden y
  durabilidad; agruparlos volvería el diff irrevisable y resolvería cinco preguntas de esquema
  detrás de una sola aprobación. Registrado como **F-1**.
- **NG-2 — No se crea ningún `StorageEngine`, abstracción de dialecto SQL, ORM, motor genérico de
  repositorios ni toolkit de backends.** **Motivo**: una abstracción compartida extraída de dos
  backends es una conjetura. La auditoría previa de extensibilidad de persistencia concluyó que la
  duplicación entre backends aquí es barata y que la abstracción aún no está ganada; tres
  implementaciones concretas de un puerto son la evidencia que una extracción futura necesitaría,
  no un motivo para preconstruirla (**F-2**).
- **NG-3 — Ningún backend más allá de Memory, PostgreSQL y Stoolap.** Ni Oracle, ni MySQL, ni
  SQLite, ni ningún otro motor, y ningún punto de extensión que anticipe uno. **Motivo**: la matriz
  de backends soportados es exactamente esos tres, de forma permanente. Anticipar un cuarto es la
  abstracción que NG-2 rechaza, con otro nombre.
- **NG-4 — CORE-PERSIST-A2 no se ejecuta.** `EffectStateStore`, `EffectDedupStore` y
  `RetentionMaintenance` permanecen en `crates/runtime/src/effects/store.rs`. **Motivo**: reubicar
  un puerto es una decisión de propiedad con su propio radio de impacto, ya identificada como cambio
  propio (**F-3**).
- **NG-5 — `crates/persistence-api/` no se edita en absoluto.** Ningún método, límite, supertrait,
  cuerpo por defecto ni variante de error de ningún puerto cambia (D-7). **Motivo**: un backend que
  exige mover su contrato es un backend que no encaja en el contrato, y la exploración confirmó que
  este sí encaja.
- **NG-6 — `crates/persistence/` no se renombra ni se modifica.** Sin renombrado a
  `ego-persistence-postgres`, sin cambios de SQL, migraciones ni índices. **Motivo**: confirmado de
  bajo costo pero opcional por la auditoría previa, y por completo independiente de que exista o no
  un adaptador Stoolap (**F-4**).
- **NG-7 — No se reutiliza, comparte, extrae ni importa ningún detalle de implementación de
  PostgreSQL** (D-11). **Motivo**: los dos adaptadores comparten un contrato, no un linaje.
- **NG-8 — No entra Testcontainers, Docker ni ninguna dependencia de contenedores al workspace
  raíz.** **Motivo**: `integration-tests/Cargo.toml:1-12` convierte la raíz sin Docker en una
  garantía estructural, no en una convención; Stoolap es embebido y no necesita nada (D-10).
- **NG-9 — Ninguna implementación existente cambia su comportamiento para igualar a la nueva.** Si
  el harness compartido expone una divergencia genuina en `InMemoryRepository` o
  `PostgreSQLRepository`, se registra como deuda nombrada con seguimiento, no se corrige dentro de
  este diff. **Motivo**: una corrección de exactitud sobre un adaptador publicado merece sus propios
  tests, su propia revisión de radio de impacto y su propio revisor: la misma regla que
  CORE-PERSIST-B aplicó a su KD-5.

## Capacidades

### Capacidades nuevas

- `persistence-stoolap-adapter`: el contrato observable de que existe un `Repository<A>` respaldado
  por Stoolap; que delimita agregados por tenant con el ámbito systemwide aislado de todo tenant
  concreto e igual a sí mismo; que impone concurrencia optimista reportando cada carrera perdida
  como conflicto; que un save confirmado sobrevive al reinicio del proceso; y que su comportamiento
  observable desde fuera es indistinguible del de las implementaciones en memoria y PostgreSQL para
  cada escenario que cubre el harness compartido.

### Capacidades modificadas

- **Ninguna prevista.** `persistence-api-surface` queda intacta (NG-5, R6). `foundation-integrity`
  no requiere cambio de matriz: `infrastructure → domain` ya está permitido (`layers.toml:10`), así
  que la nueva entrada en `layers.toml` *satisface* el requisito de completitud existente en lugar
  de modificarlo. La spec de `testkit` no enumera harnesses de conformidad individuales, así que
  añadir uno no requiere delta. Los tres se listan para que la fase de spec **confirme** en lugar de
  asumir; si encuentra un requisito existente que deba cambiar, eso pasa a ser una pregunta
  bloqueante, no una edición silenciosa.

## Enfoque

Crear el crate con el conjunto de dependencias de D-3; definir su esquema propio con la columna
centinela de tenant y el índice único; implementar `save` como una transacción real —leer la versión
actual, comparar en Rust, escritura condicional guardada por versión, todo conflicto plegado a
`PersistenceError::Conflict`— con `load` y `delete` como sentencias delimitadas ordinarias; abrir la
base de datos solicitando explícitamente commit durable.

El orden importa para la revisabilidad, y el harness va primero: escribir el harness de conformidad
de `Repository` en `ego-testkit` y dejarlo en verde contra las dos implementaciones *existentes*
antes de que exista una sola línea de código Stoolap. Esa secuencia es lo que convierte al harness de
documentación en juez: queda calibrado contra implementaciones conocidas como buenas, de modo que un
fallo posterior de Stoolap es inequívocamente del adaptador, y cualquier divergencia que exponga
entre Memory y PostgreSQL emerge como deuda nombrada (NG-9) y no como un fallo confuso del adaptador
nuevo. Después el adaptador, y después la tercera ejecución del harness.

## Requisitos de aceptación

Cada uno es verificable de forma independiente y funciona además como criterio de éxito del cambio.

- [x] **R1 — El adaptador existe y satisface el contrato.** `StoolapRepository<A, F>` implementa
      `ego_persistence_api::persistence::Repository<A>` y pasa íntegro el harness de IS-5.
- [x] **R2 — La concordancia entre backends se prueba, no se afirma.** El harness idéntico corre en
      verde contra `InMemoryRepository`, `PostgreSQLRepository` y `StoolapRepository`: un harness,
      tres sujetos, sin variantes por backend ni escenarios omitidos.
- [x] **R3 — La unicidad systemwide se sostiene de verdad.** Dos saves del mismo `aggregate_id` bajo
      el ámbito systemwide producen una fila y aritmética de versión correcta: el modo de fallo que
      una columna de tenant anulable habría permitido (D-5) se prueba ausente, no se argumenta
      ausente.
- [x] **R4 — El centinela nunca escapa del adaptador.** Ningún valor visible al llamador, mensaje de
      error ni tipo retornado expone la codificación; el contrato `Option<&str>` de `Repository` se
      comporta de forma idéntica en las tres implementaciones, incluido `MissingTenant` ante
      `Some("")`.
- [x] **R5 — La durabilidad queda fijada por un test.** Un save confirmado sobrevive a un ciclo de
      cierre y reapertura, y el modo de sincronización configurado del adaptador se afirma en lugar
      de asumirse (D-6).
- [x] **R6 — Sin cambio de contrato.** `crates/persistence-api/**` queda sin modificar; ningún
      conjunto de métodos, límite, supertrait, cuerpo por defecto ni variante de error de ningún
      puerto cambia.
- [x] **R7 — Independencia de backend.** `crates/persistence-stoolap/` no nombra `sqlx`, `PgPool`,
      `ego-persistence`, ninguna migración de PostgreSQL ni ningún símbolo de PostgreSQL en su
      manifiesto ni en sus fuentes; `crates/persistence/**` no aparece en el diff.
- [x] **R8 — Integridad de dependencias y capas.** El `Cargo.toml` del crate nombra exactamente el
      conjunto de D-3; `cargo run -p xtask -- verify-layers` pasa sin violaciones nuevas y sin
      editar la matriz.
- [x] **R9 — El workspace raíz sigue libre de Docker.** `cargo test --workspace` pasa sin ningún
      runtime de contenedores disponible, y ninguna dependencia de Testcontainers aparece en el
      workspace raíz.
- [x] **R10 — Contención del alcance.** No existe implementación Stoolap de ningún puerto distinto
      de `Repository`, y no se introduce ninguna abstracción `StorageEngine`/dialecto/ORM/motor
      genérico.
- [x] **R11 — El comportamiento existente no cambia.** `InMemoryRepository` y
      `PostgreSQLRepository` son idénticos en comportamiento; cualquier divergencia que exponga el
      harness se registra como deuda (NG-9), no se corrige aquí.
- [x] **R12 — Fidelidad de conflictos.** Tanto un `expected_version` obsoleto como una carrera real
      con un escritor concurrente se manifiestan como `PersistenceError::Conflict`, y no se añade
      ninguna variante de error.
- [x] **R13 — Propiedad del harness.** El harness se declara una sola vez, en `ego-testkit`, y cada
      backend lo consume en lugar de copiarlo.
- [x] **R14 — El trabajo diferido se nombra, no se resuelve en silencio.** F-1 a F-4 quedan
      registrados con responsables y prerrequisitos, y nada de esas fronteras se toca.

## Deuda conocida (registrada, no corregida)

- **KD-1** — `Repository` no ha tenido harness de conformidad compartido desde que CORE-PERSIST-B lo
  registró (su KD-4). Este cambio lo cierra solo para `Repository`; `Snapshot`, `OffsetStore` y
  `DedupStore` siguen sin cobertura.
- **KD-2** — El proveedor Stoolap existente del effect-store abre `file://{path}` sin parámetro de
  sincronización (`crates/effect-store/src/stoolap/mod.rs:175`), de modo que corre con el defecto sin
  fsync de Stoolap. **Observado, ni juzgado ni modificado aquí**: es otro puerto, otro requisito de
  durabilidad y otro revisor. Se registra para que la pregunta se haga, no para darla por respondida.
- **KD-3** — Cualquier divergencia Memory/PostgreSQL que exponga el nuevo harness (NG-9), si
  aparece.

## Seguimientos nombrados

- **F-1** — Stores adicionales respaldados por Stoolap (`EventStore`, `Snapshot`,
  `OperationReservationStore`, `OffsetStore`, `DedupStore`), cada uno como cambio propio con sus
  propias decisiones de esquema (NG-1).
- **F-2** — Revisar la abstracción de backend *solo después* de que existan tres implementaciones
  concretas de un segundo puerto y la duplicación esté medida en lugar de predicha (NG-2).
- **F-3** — **CORE-PERSIST-A2**: reubicar `EffectStateStore`, `EffectDedupStore` y
  `RetentionMaintenance` fuera de `crates/runtime/src/effects/store.rs` hacia `ego-persistence-api`
  (NG-4).
- **F-4** — Renombrado opcional `ego-persistence` → `ego-persistence-postgres`, confirmado de bajo
  costo y no bloqueante por la auditoría previa (NG-6).

## Áreas afectadas

| Área | Impacto | Descripción |
|------|---------|-------------|
| `crates/persistence-stoolap/` | Nuevo | El crate completo: `Cargo.toml`, esquema, `StoolapRepository` (IS-1..IS-4) |
| `crates/testkit/src/` | Modificado | Nuevo harness compartido de conformidad de `Repository` + export en `lib.rs` (IS-5, D-9) |
| `crates/persistence-memory/`, `crates/persistence/` | Solo tests | Dev-dependency y un target de test que invoca el harness compartido; **sin cambio de fuente** (IS-6, R11) |
| `integration-tests/` | Modificado | Ejecución PostgreSQL del harness compartido (D-10, IS-6) |
| `layers.toml`, `Cargo.toml` raíz | Modificado | Una entrada de capa, un miembro del workspace (IS-1) |
| `xtask/src/layers.rs` | Intacto | No se requiere cambio de matriz (D-2) |
| `crates/persistence-api/` | Intacto | Sin cambio de contrato (NG-5, R6) |
| `crates/runtime/`, `crates/effect-store/` | Intactos | Diferidos (NG-4, KD-2) |
| `openspec/specs/persistence-stoolap-adapter/spec.md` | Nuevo | Delta según IS-6 |

## Riesgos

| ID | Riesgo | Probabilidad | Mitigación |
|----|--------|--------------|------------|
| RK-1 | **La codificación centinela se filtra al comportamiento visible del llamador**, y el adaptador Stoolap deja de ser sutilmente el mismo `Repository` que los otros dos | Media | D-5 la confina al almacenamiento; R4 convierte la no filtración en aserción verificable; R2 hace que el *mismo* harness juzgue a los tres, así que una filtración hace fallar un test en lugar de sobrevivir como comentario |
| RK-2 | **La durabilidad se degrada en silencio al defecto de Stoolap**, exactamente lo que resulta fácil de heredar, con precedente dentro del árbol (KD-2) | Media | D-6 la convierte en decisión con revisor nombrado; R5 la fija con un test que falla ante el defecto, en vez de un comentario de código que lo pida amablemente |
| RK-3 | **El conflicto de reclamo de escritura MVCC de Stoolap emerge como error genérico opaco**, se clasifica mal como `Internal` y nunca lo reintenta un llamador que habría tenido éxito | Media | D-7 pliega todo conflicto de escritura en `Conflict`; R12 exige demostrar que tanto una versión obsoleta como una carrera concurrente real lo producen. `design.md` es dueño del predicado exacto de clasificación |
| RK-4 | **El harness se escribe según lo que casualmente hace el adaptador Stoolap**, y pasa trivialmente | Media | El Enfoque fija el orden: el harness se escribe y se prueba en verde contra las dos implementaciones existentes *antes* de que exista código Stoolap |
| RK-5 | **Aparece una divergencia Memory/PostgreSQL a mitad del cambio**, tentando a corregir en el mismo diff un adaptador publicado | Media | NG-9 y R11 lo convierten en seguimiento por regla (KD-3). Si la divergencia impide siquiera escribir el harness, eso es una pregunta bloqueante para el usuario, no una corrección improvisada |
| RK-6 | **Deriva de alcance hacia una abstracción de backend**: tres implementaciones de un puerto es justo el momento en que el patrón parece extraíble | Media | NG-2, NG-3, R10 y la propia regla de maduración arquitectónica del workspace: un principio necesita 2–3 recurrencias independientes antes de promoverse. Un puerto es un solo dato (F-2) |
| RK-7 | **Presupuesto de revisión.** Un crate nuevo, un harness nuevo y tres puntos de invocación excederán el presupuesto de 400 líneas | Alta | Pronosticado, no oculto. `sdd-tasks` debería rebanar así: (1) harness compartido + ejecuciones Memory/PostgreSQL, (2) el crate con esquema y `save`/`load`/`delete`, (3) la ejecución Stoolap del harness más el test de durabilidad. Cada rebanada deja el workspace compilando y toda rebanada previa en verde |

## Plan de reversión

**Un solo commit de revert, en cualquier momento, sin ruptura externa.**

Nada fuera del nuevo crate depende de él: `ego-persistence-stoolap` es aditivo, ningún crate
existente adquiere una dependencia no-dev de él, y ningún archivo fuente existente cambia de
comportamiento. Revertir consiste en eliminar `crates/persistence-stoolap/`, quitar el miembro del
workspace y la entrada de `layers.toml`, y retirar el harness de `ego-testkit` junto con sus tres
puntos de invocación. `xtask/src/layers.rs` nunca cambió, así que no hay estado de compuerta que
deshacer, y `crates/persistence-api/` y `crates/persistence/` nunca se tocaron.

Esto se mantiene **a mitad de camino**, que es lo que hace segura la partición de RK-7: un
STOOLAP-S1 parcialmente aterrizado es un workspace que ganó un harness de conformidad y quizá un
crate nuevo sin uso. Ninguno de los dos es alcanzable desde una ruta de cableado de producción,
porque nada cablea el adaptador: un despliegue opta por él añadiendo la dependencia, y ninguno lo
hace todavía.

Ningún dato, esquema ni migración de ningún store existente está involucrado en ninguna dirección. El
adaptador crea únicamente sus propias tablas, en su propio archivo de base de datos.

## Dependencias

- `persistence-api-surface` (publicada, CORE-PERSIST-A) — el contrato `Repository<A>` que este crate
  implementa, consumido por completo sin cambios.
- `persistence-memory-adapter` (publicada, CORE-PERSIST-B) — aporta el segundo sujeto del harness.
- `foundation-integrity` (archivada) — FR-001 (completitud), FR-002 (dirección), FR-003 (sin
  ciclos), FR-005 (compilación aislada), consumidas sin cambios.
- Regla de diseño de `openspec/config.yaml` "No circular dependencies between crates" — respetada por
  construcción (D-2, D-3).
- `stoolap 0.4.0` — ya fijado en `Cargo.lock` (checksum `420d8bd6…`) vía la feature opcional
  `stoolap` de `ego-effect-store`. **No se introduce ningún crate, servicio ni infraestructura
  externa nueva.**
- Los cinco hallazgos de capacidad de la exploración de STOOLAP-S1 (Engram `sdd/stoolap-s1/explore`).

## Ronda de preguntas de propuesta

Esta propuesta se produjo sin ronda interactiva. Cinco preguntas de producto la afinarían; hasta que
se respondan, aplica el supuesto declarado.

1. **¿Quién es el cliente de un `Repository` sobre Stoolap?** ¿Despliegues de un solo nodo o en el
   borde que quieren agregados durables sin operar PostgreSQL, o principalmente pruebas más rápidas y
   realistas que las de memoria? *Supuesto: despliegues durables de un solo nodo, que es la razón por
   la que D-6 trata la durabilidad de commit como innegociable y no como parámetro ajustable.*
2. **¿Stoolap es un backend de producción soportado, o soportado pero no recomendado?** La respuesta
   cambia lo que la spec debe decir sobre expectativas operativas, respaldos y límites de
   concurrencia. *Supuesto: soportado, con sus características de concurrencia de un solo nodo
   declaradas con honestidad en la spec.*
3. **RK-5 / NG-9** — si el harness compartido expone una divergencia real entre las dos
   implementaciones *existentes*, ¿este cambio debe detenerse y escalar, o registrarla y continuar?
   *Supuesto: registrar y continuar, salvo que la divergencia impida escribir un escenario del
   harness.*
4. **KD-2** — ¿el defecto sin fsync de Stoolap en el effect-store es intencional para ese puerto, o
   una herencia no advertida que merece su propio seguimiento? *Supuesto: fuera de alcance en
   cualquier caso; registrado como observación, no agendado.*
5. **Secuencia de F-1** — después de `Repository`, ¿qué store se gana el siguiente adaptador Stoolap,
   y existe algún despliegue que necesite el conjunto completo antes de que algo de esto sea útil?
   *Supuesto: aquí no se agenda ninguno; `Repository` se sostiene solo como rebanada completa y útil
   por sí misma.*
