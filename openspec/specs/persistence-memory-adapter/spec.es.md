# Spec: `persistence-memory-adapter` (Capability Nueva)

> Documento acompañante para revisión. La fuente de verdad canónica es `spec.md` (identificadores
> de requisito y escenarios 1:1). Fuente del contenido de los requisitos: los Requisitos de
> Aceptación R1-R18 del `proposal.md` de CORE-PERSIST-B. El contrato de esta capability es
> puramente estructural — una reubicación/re-export puro con cero comportamiento nuevo — por lo
> que cada escenario se formula en términos de ubicación de la declaración, resolución en tiempo
> de compilación e identidad, no de comportamiento nuevo en tiempo de ejecución.

## Propósito

El contrato observable de que las implementaciones en memoria del workspace para los puertos de
persistencia propiedad del dominio tienen exactamente un crate propietario,
`ego-persistence-memory`; que cada ruta que hoy resuelve una de ellas sigue resolviendo al mismo
ítem después de este cambio; que ningún puerto gana, pierde o cambia una implementación; y que la
clasificación de durabilidad no cambia. Esta capability no cubre los adaptadores de PostgreSQL,
un arnés de conformidad, ni ningún puerto, método o comportamiento nuevo.

## Requisitos

### Requisito: R1 — Propiedad Canónica

El sistema DEBE asegurar que cada una de las siete implementaciones reubicadas
(`InMemoryEventStore` + `InMemoryEventStoreUnitOfWork`, `InMemoryRepository`,
`InMemorySnapshotStore`, `InMemoryReadSideStore` + `paginate`, `InMemoryOffsetStore`,
`InMemoryDedupStore`, `InMemoryOperationReservationStore`) resuelve desde exactamente un crate
declarante, `ego-persistence-memory`, y no está declarada en ningún otro lugar.

#### Escenario: Cada implementación tiene exactamente un crate declarante

- DADAS las siete implementaciones reubicadas después de este cambio
- CUANDO se busca en el workspace sus declaraciones por nombre
- ENTONCES cada nombre está declarado exactamente una vez, en `ego-persistence-memory`

#### Escenario: Los crates desalojados ya no la declaran

- DADO que `ego-infrastructure` y `ego-testkit` contenían las declaraciones previas al cambio
- CUANDO se completa la reubicación
- ENTONCES esos crates contienen solo re-exports `pub use` en las rutas antiguas, no
  declaraciones

### Requisito: R2 — No Se Introduce Ninguna Implementación Canónica Duplicada

La reubicación DEBE crear cero declaraciones nuevas; el conteo de bloques `impl <Puerto> for`
por cada puerto reubicado DEBE permanecer sin cambios en todo el workspace.

#### Escenario: El conteo de bloques impl es estable a través de la reubicación

- DADO el conteo, en todo el workspace, de bloques `impl <Puerto> for` para cada uno de los ocho
  puertos reubicados antes de este cambio
- CUANDO se toma el mismo conteo después de este cambio
- ENTONCES los conteos son idénticos

### Requisito: R3 — Los Fakes de Prueba Nombrados No Son Promovidos

`FakeDurableOffsetStore` y `FakeDurableDedupStore` DEBEN permanecer declarados en
`examples/reference-app`, idénticos byte a byte, y NO DEBEN aparecer en ningún lugar de
`ego-persistence-memory`.

#### Escenario: Los fakes permanecen en el ejemplo, sin editar

- DADOS `FakeDurableOffsetStore` y `FakeDurableDedupStore` tal como están declarados en
  `examples/reference-app/src/read_side/store.rs` antes de este cambio
- CUANDO este cambio se completa
- ENTONCES ambos siguen declarados ahí, idénticos byte a byte, y ninguno está declarado en
  `ego-persistence-memory`

### Requisito: R4 — Lo Faltante Sigue Visiblemente Faltante

`ProjectionStateStore` DEBE tener cero implementaciones después de este cambio, y no DEBE
añadirse ningún stub, placeholder ni implementación `todo!()`.

#### Escenario: Un puerto muerto sigue muerto

- DADO que `ProjectionStateStore` tiene cero implementaciones antes de este cambio
- CUANDO se busca en el workspace implementaciones después de este cambio
- ENTONCES sigue teniendo cero implementaciones, y no existe ningún stub o placeholder en
  `ego-persistence-memory` ni en ningún otro lugar

### Requisito: R5 — Preservación del Comportamiento

El cuerpo de cada tipo reubicado — incluyendo la resolución de tenant, la estrategia de bloqueo,
la aritmética de conflicto de versión y el manejo de fail-closed para tenant vacío — DEBE ser
textualmente idéntico a su forma previa al cambio, salvo la ruta de módulo y las líneas `use`.

#### Escenario: Un cuerpo reubicado es un diff de solo ruta de módulo e imports

- DADO el archivo fuente previo al cambio de una implementación reubicada
- CUANDO se compara contra su ubicación posterior al cambio en `ego-persistence-memory`
- ENTONCES las únicas diferencias son la declaración de ruta de módulo y las líneas `use`

#### Escenario: El manejo fail-closed de tenant vacío sobrevive a la reubicación

- DADO el comportamiento fail-closed previo al cambio de `InMemoryReadSideStore` ante un tenant
  vacío
- CUANDO el store se ejercita después de la reubicación con un tenant vacío
- ENTONCES sigue fallando de forma cerrada, con la ruta de error idéntica

### Requisito: R6 — Preservación de Durabilidad y de Producción

Ningún tipo reubicado DEBE declarar `is_durable()`; `presence_alone_is_not_durability` y ambas
pruebas `try_build_rejects_explicit_in_memory_*` DEBEN pasar sin modificar, rechazando aún los
stores en memoria bajo `Profile::Production`.

#### Escenario: El perfil de producción sigue rechazando un store en memoria

- DADO `EntityRuntimeBuilder` configurado con `Profile::Production` y un `InMemoryEventStore` o
  `InMemorySnapshotStore` reubicado explícito
- CUANDO el runtime intenta construirse
- ENTONCES falla, nombrando la capability no durable, exactamente como antes de la reubicación

#### Escenario: Ningún tipo reubicado sobrescribe is_durable

- DADAS las siete implementaciones reubicadas
- CUANDO cada una se inspecciona en busca de una sobrescritura `fn is_durable`
- ENTONCES ninguna existe, y cada una usa por defecto el `false` del trait

### Requisito: R7 — Neutralidad de Backend

`ego-persistence-memory` NO DEBE contener ninguna referencia a ningún backend — ningún tipo,
dependencia o feature flag de `sqlx`, PostgreSQL, Stoolap, HTTP o Kafka — y NO DEBE ofrecer
ninguna superficie de selección de backend.

#### Escenario: El grafo de dependencias del crate está libre de backend

- DADOS `crates/persistence-memory/Cargo.toml` y su árbol de fuentes
- CUANDO se inspeccionan ambos
- ENTONCES ninguno nombra ni importa `sqlx`, un cliente de PostgreSQL/Stoolap, un cliente HTTP o
  un cliente Kafka, y ningún feature flag selecciona un backend

### Requisito: R8 — Consolidación del Lado de Lectura

`InMemoryOffsetStore` e `InMemoryDedupStore` DEBEN estar declarados en `ego-persistence-memory` y
ya no en `examples/reference-app`; el ejemplo DEBE consumirlos como una dependencia ordinaria.

#### Escenario: El ejemplo ya no declara los dos stores de lado de lectura

- DADO `examples/reference-app/src/read_side/store.rs` antes de este cambio
- CUANDO este cambio se completa
- ENTONCES `InMemoryOffsetStore` e `InMemoryDedupStore` están declarados solo en
  `ego-persistence-memory`, y el ejemplo los importa como una dependencia

### Requisito: R9 — Re-Exports de Compatibilidad en Cada Ruta Antigua

Cada ruta en la MATRIZ DE RE-EXPORTS DE COMPATIBILIDAD DEBE seguir resolviendo, sin editar, al
mismo ítem — probado en tiempo de compilación sobre la lista completa, no por muestreo. Los
seis archivos consumidores downstream confirmados DEBEN compilar con código fuente idéntico
byte a byte.

#### Escenario: Una ruta antigua resuelve al ítem reubicado idéntico

- DADO `ego_infrastructure::persistence::in_memory::InMemoryEventStore` como ruta de import
  previa al cambio
- CUANDO el workspace compila después de este cambio
- ENTONCES sigue resolviendo, y el tipo resuelto coacciona por identidad con
  `ego_persistence_memory::persistence::event_store::InMemoryEventStore`

#### Escenario: Los seis archivos consumidores confirmados compilan sin editar

- DADOS `crates/infrastructure/tests/in_memory_event_store_conformance.rs`,
  `crates/infrastructure/tests/commit_publishes_atomically.rs`,
  `examples/reference-app/src/lib.rs`, `crates/transport/tests/operation_key_extractor.rs`, y los
  dos archivos
  `crates/service-sdk/tests/{retention_worker_lifecycle,cross_tenant_reservation_isolation}.rs`
- CUANDO el workspace se reconstruye después de este cambio
- ENTONCES todos compilan con código fuente idéntico byte a byte al de antes del cambio

### Requisito: R10 — Propiedad de Implementación Única Por Puerto Reubicado

Para `EventStore`, `EventStoreUnitOfWork`, `Repository`, `Snapshot`, `ReadSideStore`,
`OffsetStore`, `DedupStore` y `OperationReservationStore`, `ego-persistence-memory` DEBE ser el
único propietario en memoria de propósito general; las únicas otras declaraciones que
sobreviven DEBEN ser los dos duplicados nombrados de `persistent-entity` y los fakes de prueba
declarados.

#### Escenario: No existe una tercera implementación de propósito general

- DADOS los ocho puertos reubicados
- CUANDO se busca en el workspace implementaciones de propósito general (no fakes, no
  `persistent-entity`) de cada uno
- ENTONCES se encuentra exactamente una para cada uno, y está declarada en
  `ego-persistence-memory`

### Requisito: R11 — Integridad de Dependencias

El `Cargo.toml` de `ego-persistence-memory` DEBE nombrar exactamente `ego-persistence-api` y
`ego-domain` como dependencias `path` del workspace y nada más; NO DEBE nombrar
`ego-application`, `ego-runtime`, `ego-infrastructure`, `ego-persistence`, `ego-testkit`,
transport, ni ninguna dependencia de ejemplo. `cargo run -p xtask -- verify-layers` DEBE pasar
sin ninguna violación nueva ni edición de la matriz.

#### Escenario: El Cargo.toml nombra exactamente dos dependencias path del workspace

- DADO `crates/persistence-memory/Cargo.toml`
- CUANDO se inspecciona
- ENTONCES nombra exactamente `ego-persistence-api` y `ego-domain` como dependencias `path` del
  workspace, y ningún otro crate del workspace

#### Escenario: La puerta de capas pasa sin edición de la matriz

- DADO que `layers.toml` gana la única entrada `ego-persistence-memory = "foundation"` y
  `xtask/src/layers.rs` permanece intacto
- CUANDO se ejecuta `cargo run -p xtask -- verify-layers`
- ENTONCES pasa sin ninguna violación nueva

### Requisito: R12 — Integridad de Alcance de Effects

`crates/runtime/` y `crates/effect-store/` DEBEN permanecer sin modificar; `InMemoryEffectStore`
y sus tres puertos DEBEN ser idénticos byte a byte; el límite D-9 de CORE-PERSIST-A DEBE
permanecer intacto.

#### Escenario: Los crates de effect-store permanecen intactos

- DADOS `crates/runtime/` y `crates/effect-store/` antes de este cambio
- CUANDO este cambio se completa
- ENTONCES ambos son idénticos byte a byte a antes, incluyendo `InMemoryEffectStore` y
  `EffectStateStore`/`EffectDedupStore`/`RetentionMaintenance`

### Requisito: R13 — Sin Refactor de PostgreSQL

Cero archivos de SQL, migración, esquema o `crates/persistence/` DEBEN aparecer en el diff.

#### Escenario: El diff no toca ningún archivo propiedad de PostgreSQL

- DADO el diff completo de este cambio
- CUANDO se inspecciona en busca de archivos de SQL, migración, esquema o `crates/persistence/`
- ENTONCES ninguno aparece

### Requisito: R14 — Sin Expansión del Marco de Conformidad

Ningún arnés de conformidad DEBE ser añadido, extendido o generalizado; `assert_event_store_conformance`
y las pruebas de lease de reserva DEBEN conservar su forma y ubicación actuales.

#### Escenario: El arnés de conformidad no cambia

- DADOS `assert_event_store_conformance` y las pruebas de lease de reserva antes de este cambio
- CUANDO este cambio se completa
- ENTONCES ambos conservan su forma y ubicación de crate previas al cambio

### Requisito: R15 — Sin Rediseño de Contrato o Trait

`crates/persistence-api/src/**` DEBE permanecer sin modificar; el conjunto de métodos, bounds,
supertraits, cuerpos por defecto u object-safety de ningún puerto DEBE cambiar.

#### Escenario: El crate de puertos es idéntico byte a byte

- DADO `crates/persistence-api/src/**` antes de este cambio
- CUANDO este cambio se completa
- ENTONCES es idéntico byte a byte, y el conjunto de métodos, bounds, supertraits, cuerpos por
  defecto u object-safety de ningún puerto difiere

### Requisito: R16 — Ningún Doble de Prueba de Ningún Tipo Es Promovido

`TestClock` DEBE permanecer en `ego-testkit`, y ningún doble local a `#[cfg(test)]` o a
`tests/` DEBE moverse a `ego-persistence-memory`.

#### Escenario: TestClock y los dobles locales permanecen en su lugar

- DADOS `TestClock` y cada doble local a `#[cfg(test)]` o a `tests/` antes de este cambio
- CUANDO este cambio se completa
- ENTONCES ninguno de ellos está declarado en `ego-persistence-memory`, y `TestClock` permanece
  en `ego-testkit`

### Requisito: R17 — Los Dos Duplicados de `persistent-entity` Son Deuda Nombrada, No Resuelta en Silencio

Tanto el duplicado de capability aditiva `EventStore`/`EventStoreUnitOfWork` como el duplicado de
`Snapshot` que ignora el tenant DEBEN registrarse como KD-6 y KD-5 respectivamente, cada uno con
un propietario de seguimiento nombrado (F-6, F-5), y ninguno DEBE ser movido, fusionado,
corregido, ni abordado parcialmente.

#### Escenario: Ambos duplicados están nombrados, no movidos

- DADOS el `InMemoryEventStore`/`StagingUnitOfWork` y el `InMemorySnapshotStore` de
  `persistent-entity`
- CUANDO este cambio se completa
- ENTONCES ambos permanecen exactamente donde estaban, y la documentación del cambio los nombra
  como KD-6/F-6 y KD-5/F-5 respectivamente

#### Escenario: El defecto de aislamiento de tenant no se corrige dentro de esta reubicación

- DADO que el `InMemorySnapshotStore` de `persistent-entity` ignora `tenant_id` (defecto
  confirmado)
- CUANDO este cambio se completa
- ENTONCES el defecto sigue reproduciéndose idénticamente, y se registra como deuda nombrada
  (KD-5) en lugar de corregirse

### Requisito: R18 — El Límite de Effect-Store Es Deuda Nombrada, No Resuelta en Silencio

El cambio futuro que reubicaría los puertos de effect-store y consolidaría
`InMemoryEffectStore` DEBE ser nombrado (CORE-PERSIST-E) con su prerequisito declarado (la
reubicación de puertos antes de la consolidación de implementación), y nada de ese límite DEBE
ser tocado por este cambio.

#### Escenario: El límite está documentado, no cruzado

- DADO el límite D-9/D-10 entre `ego-persistence-memory` y los puertos de effect-store de
  `ego-runtime`
- CUANDO se inspecciona la documentación de este cambio
- ENTONCES nombra a CORE-PERSIST-E como el seguimiento y declara que la reubicación de puertos
  debe llegar primero, y ningún archivo de effect-store es tocado
