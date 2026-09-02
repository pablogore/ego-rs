# Propuesta: PROD-014B — Almacenes de Lado de Lectura Durables en PostgreSQL

> Compañero de revisión en español. Fuente de verdad canónica: `proposal.md` (identificadores 1:1).

## Objetivo

PROD-014A entregó una compuerta `Profile::Production` que rechaza una proyección de lado de
lectura cuyo par de progreso es volátil — y no dejó nada en el árbol capaz de satisfacerla.
PROD-014B aporta la implementación faltante: un par `OffsetStore` + `DedupStore` sobre PostgreSQL
cuyo estado sobrevive al reinicio del proceso, conforme al SPI tal como existe hoy.

## Intención

Un host de producción que adopte PROD-014A tiene hoy exactamente dos opciones dentro del árbol:
fallar la compuerta, o registrar un doble de prueba (`FakeDurableOffsetStore` /
`FakeDurableDedupStore`, `examples/reference-app/src/read_side/store.rs:150-307`) que declara una
durabilidad que no tiene. La propia ruta de producción de `examples/reference-app` no toma
ninguna de las dos: pasa `None` para el progreso de lado de lectura, con un comentario explícito
"PROD-014A F-1" (`examples/reference-app/src/main.rs:109-114`).

Una compuerta sin implementación que la satisfaga es un rechazo, no una garantía. PROD-014B
salda el seguimiento nombrado F-1 de PROD-014A y nada más: sin cambios de SPI, sin cambios de
compuerta, sin cambios de registro.

También arrastra algo que no debe perderse detrás de la palabra "durable". El almacenamiento
durable de registros de deduplicación **no** es procesamiento exactamente-una-vez, y PROD-014B no
lo convierte en tal por estar respaldado por PostgreSQL. `ego-rs` apunta a producción
distribuida, así que la afirmación honesta es una restricción de adopción, no una advertencia
vaga: **PROD-014B es adoptable en Producción solo bajo escritor-único-por-`(projection_id, tag,
tenant)`, hasta que exista un mecanismo de reserva atómica o imposición equivalente.** Ese
límite se declara abajo como limitación nombrada y aceptada, con su propio criterio de éxito —
ver **Garantías y Limitaciones Nombradas**.

## Decisiones Activas

| ID | Decisión | Justificación |
|----|----------|---------------|
| D-1 | **Dos tablas, no una.** `projection_offsets` con identidad `(projection_id, tag, tenant)`; `projection_dedup` con identidad `(projection_id, tag, event_id)`. El tenant **no** forma parte de la identidad de deduplicación | Los dos SPIs tienen formas de clave genuinamente distintas y sus docs lo dicen: `OffsetStore` es "independiente por tupla (projection_id, tag, tenant)" (`crates/domain/src/read_side/offset.rs:51-83`); el "alcance de deduplicación: (projection_id, tag, event_id)" de `DedupStore` (`dedup.rs:20-51`). Las tuplas `OffsetKey`/`DedupKey` de la reference-app confirman ambas (`read_side/store.rs:143-148`). El par estado/dedup análogo de `crates/effect-store` también son dos tablas |
| D-2 | **Ambas columnas de tenant son `NOT NULL`.** El patrón de tenant nulable de los adaptadores de lado de escritura (`tenant_id IS NOT DISTINCT FROM $N` más índices únicos parciales, `crates/persistence/src/postgres/snapshot.rs:63-137`) deliberadamente **no** se copia | El parámetro del SPI de lado de lectura es `tenant: &str`, nunca `Option<&str>`. El concepto de tenant global/"systemwide" del framework existe solo para almacenes de lado de escritura (`crates/domain/src/persistence/tenant.rs:29-35`) y está estructuralmente ausente del SPI de lado de lectura. Todos los sitios de llamada del lado de lectura están concretamente acotados por tenant (`crates/runtime/src/read_side/scheduler.rs:840-882`; `examples/reference-app/src/read_side/mod.rs:199` descarta cualquier tag sin tenant decodificable antes de que llegue a una llamada al almacén). Una columna nulable modelaría un estado que el SPI no puede expresar |
| D-3 | **`write_offset` es un upsert plano.** Sin token CAS, sin offset previo esperado, sin verificación de orden | `write_offset` (`offset.rs:77-83`) no recibe valor esperado y su documentación describe una sobrescritura plana; el único llamador (`crates/domain/src/read_side/session.rs:91-176`) nunca verifica si su escritura ganó. Un adaptador que inventara una garantía CAS sería más estricto que el contrato que implementa, y fallaría para llamadores que el trait considera válidos |
| D-4 | **La retención de deduplicación es explícitamente ilimitada en este cambio.** Ni purga, ni TTL, ni evicción se entregan en PROD-014B, y la especificación lo dice de frente en lugar de omitirlo | La retención está indefinida en todas las capas hoy — `scheduler.rs`, `runner.rs` y `session.rs` no contienen lógica de TTL/evicción, ni relación codificada entre eliminación de deduplicación y avance del offset. El precedente del propio workspace es que la retención es una decisión deliberada y de propiedad separada: el `effect_dedup` de `crates/effect-store` tiene una ruta de limpieza separada y explícita (`crates/effect-store/src/postgres/mod.rs:285-356`). La omisión silenciosa sería la única opción inaceptable. Seguimiento F-2 |
| D-5 | **Ubicación: `crates/persistence/src/postgres/`, continuando la secuencia plana de migraciones en `013+`**, siguiendo el patrón de escritura a prueba de conflictos de `reservation.rs` (`INSERT ... ON CONFLICT DO NOTHING` / `UPDATE` condicional) | Coincide con la ubicación de todos los adaptadores existentes de la ruta dorada y no agrega aristas al grafo de dependencias: `ego-persistence` ya depende solo de `ego-domain`, donde viven ambos SPIs, y `reference-app` ya depende de `ego-persistence`. La secuencia independiente `001/002` de `crates/effect-store` se justificó en AD-10 como propiedad de un crate ya separado, no como regla general |
| D-6 | **Sin cambios de SPI, sin cambios en la lógica de la compuerta `Profile::Production`, sin cambios en `AppBuilder::read_side_progress`.** El adaptador obtiene `is_durable() -> true` y las impls genéricas de reenvío `Arc<T>` existentes gratis (`offset.rs:91-119`, `dedup.rs:59-86`) | PROD-014A entregó las tres superficies. Este cambio implementa contra ellas; modificarlas aquí lo convertiría en un segundo cambio de gobernanza disfrazado de adaptador |
| D-7 | **La brecha de concurrencia verificar-luego-actuar de `DedupStore` se arrastra como limitación contractual nombrada y aceptada — ni cerrada, ni absorbida en silencio.** Se declara en los criterios de aceptación (ver Garantías y Limitaciones Nombradas), no como nota de implementación | `seen()` y `mark_seen()` son dos métodos de trait separados (`dedup.rs:37-51`) y `ReadSideSession::execute` ejecuta el handler entre la verificación (`session.rs:116-128`) y el commit (`session.rs:142-149`). Ningún adaptador PostgreSQL puede cerrar esa ventana desde dentro de `mark_seen`. Cerrarla requiere imposición de escritor-único-por-tag a nivel de orquestación, o un método SPI atómico tipo reserve — ambos fuera de alcance. Seguimiento F-1 |
| D-8 | **Las pruebas de conformidad contra PostgreSQL real viven exclusivamente en `integration-tests/`**, vía `ego_integration_tests::isolated_database()`, coincidiendo con las 19 suites existentes `tests/infrastructure/*_postgres.rs` | `ego-rs-testing-strategy`: ninguna prueba unitaria puede alcanzar una base de datos real. `integration-tests` es un workspace separado y es el único lugar donde se admite infraestructura real |

## Compuerta de Atomicidad

**Ejecutada, y recortó el alcance dos veces.** La retención/evicción de deduplicación se
consideró y se retiró (D-4 → F-2): es entregable de forma independiente, necesita su propia
decisión de horizonte, y PROD-014B es útil y verificable sin ella. Cerrar la brecha de
concurrencia de deduplicación se consideró y se retiró (D-7 → F-1): es un cambio de
SPI/orquestación, no de adaptador, y absorberlo convertiría en silencio "implementar el par
durable" en "rediseñar el contrato de deduplicación del lado de lectura".

Lo que queda es una capacidad indivisible. El almacén de offsets por sí solo no puede satisfacer
la compuerta — esta valida el **par**. El almacén de deduplicación por sí solo tampoco. Ambos
comparten una secuencia de migraciones, una decisión de ubicación, una declaración de
durabilidad y una suite de conformidad.

**ATOMICIDAD: PASA**

## Alcance

**Frontera de un vistazo**

| | |
|---|---|
| **PROD-014B incluye** | Progreso durable de lado de lectura en PostgreSQL · migraciones · adaptadores · pruebas de conformidad · cableado de producción de la reference-app |
| **PROD-014B excluye** | Corrección multi-escritor · reserva atómica · retención/limpieza · detección de réplicas |

### En Alcance

- **IS-1** — `PostgreSQLOffsetStore`: `OffsetStore` durable sobre `projection_offsets`, identidad
  `(projection_id, tag, tenant)`, tenant `NOT NULL` (D-1, D-2), `write_offset` como upsert plano
  (D-3), `is_durable() -> true`.
- **IS-2** — `PostgreSQLDedupStore`: `DedupStore` durable sobre `projection_dedup`, identidad
  `(projection_id, tag, event_id)` con restricción UNIQUE para que un `mark_seen` repetido
  converja en lugar de fallar (D-1), `is_durable() -> true`.
- **IS-3** — Migración(es) `013+` en `crates/persistence/src/postgres/migrations/`, continuando la
  secuencia plana existente y ejecutadas por el runner existente `include_str!` + `sqlx::raw_sql`
  (D-5).
- **IS-4** — Reexportación desde `crates/persistence/src/postgres/mod.rs`, siguiendo la forma
  existente de un archivo por almacén.
- **IS-5** — Un constructor `ReadSideProgressStores::postgres(pool)` en
  `examples/reference-app/src/read_side/mod.rs`, junto a los existentes `::in_memory()` y
  `::fake_durable()`.
- **IS-6** — Recablear `examples/reference-app/src/main.rs:109-114` de `None` al par real de
  Postgres, retirando el comentario "PROD-014A F-1". **Forma parte de la Definición de Hecho, no
  es una porción postergable**: sin ello, PROD-014B entrega infraestructura que ninguna ruta de
  composición de referencia demuestra utilizable.
- **IS-7** — Una suite de conformidad contra PostgreSQL real bajo
  `integration-tests/tests/infrastructure/` usando `isolated_database()` (D-8), cubriendo:
  ida y vuelta, supervivencia al reinicio, lecturas de claves ausentes, aislamiento por tenant en
  offsets, convergencia de `mark_seen` repetido, e independencia del tenant en la identidad de
  deduplicación.
- **IS-8** — El límite de concurrencia se declara como limitación explícita y nombrada en la
  especificación y en la documentación pública de los propios adaptadores — rustdoc, README y
  documentación de configuración — en palabras legibles por una persona operadora (D-7). Esa
  documentación DEBE declarar la restricción de adopción de escritor único y NO DEBE presentar
  una configuración de proyección multi-réplica como oficialmente soportada. El código no lo
  impone; la documentación al menos debe hacerlo legible.
- **IS-9** — Deltas de especificación según la sección Capacidades.

### Fuera de Alcance

- **OOS-1** — Cualquier cambio a `OffsetStore` / `DedupStore`, a la lógica de la compuerta
  `Profile::Production`, o a `AppBuilder::read_side_progress` (D-6).
- **OOS-2** — Cerrar la brecha verificar-luego-actuar de deduplicación: ningún método atómico de
  reserva, ninguna imposición de escritor-único-por-tag, sin elección de líder, sin token de
  fencing, sin lease y sin detección de pares/réplicas (D-7 → F-1). Detectar un par concurrente
  desde dentro de un adaptador de Postgres arrastraría leases y coordinación distribuida a una
  especificación de persistencia; eso pertenece a F-1.
- **OOS-3** — Retención, TTL, purga o evicción de deduplicación de cualquier tipo (D-4 → F-2).
- **OOS-4** — Cualquier backend distinto de PostgreSQL. Stoolap, Oracle, ClickHouse, MySQL, Redis,
  RocksDB, DynamoDB, Cassandra/Scylla y SQLite quedan excluidos — una auditoría completa del
  código no encontró código productivo para ninguno, solo prosa ilustrativa en OpenSpec
  archivado.
- **OOS-5** — Un `ReadSideStore` durable (la vista de eventos que una proyección consulta).
  Heredado sin cambios de PROD-014A OOS-8 / F-2.
- **OOS-6** — Eliminar, deprecar u ocultar `InMemoryOffsetStore` / `InMemoryDedupStore` o el par
  durable falso. Siguen siendo válidos y explícitos para Dev y pruebas.
- **OOS-7** — Propiedad multi-worker, arriendo de particiones, alta disponibilidad, entrega
  exactamente-una-vez y orquestación de reconstrucción de proyecciones. Heredado sin cambios de
  PROD-014A OOS-4.
- **OOS-8** — Gobernar una proyección lanzada fuera de la raíz de composición. Heredado sin
  cambios de PROD-014A OOS-7.

## Capacidades

### Capacidades Nuevas

- `read-side-durable-progress`: el contrato observable de durabilidad del estado de progreso del
  lado de lectura — qué sobrevive a un reinicio, qué identidad tiene cada registro, qué retención
  se promete, y explícitamente qué garantía de concurrencia **no** se ofrece.

### Capacidades Modificadas

- `read-side`: el límite de concurrencia nombrado — la contabilidad durable de deduplicación no
  implica ejecución exactamente-una-vez del handler, y la prevención de doble ejecución descansa
  sobre una suposición no impuesta de escritor-único-por-tag.

Si la fase de especificación encuentra que un requisito existente ya implica alguno de estos, lo
integra en lugar de fabricar un delta.

## Enfoque

Seguir la ruta dorada ya establecida por `event_store.rs`, `snapshot.rs` y `reservation.rs`: un
archivo por almacén bajo `crates/persistence/src/postgres/`, `PgPool` por inyección en el
constructor, `is_durable()` devolviendo `true` incondicionalmente, reexportación desde
`postgres/mod.rs`, y el esquema entregado por los siguientes números de la secuencia plana de
migraciones existente.

Ambas escrituras son seguras ante conflictos por construcción, no por coordinación. `mark_seen`
es un `INSERT ... ON CONFLICT DO NOTHING` contra una identidad UNIQUE, de modo que una marca
repetida o concurrente converge a una fila sin error — la misma forma que `reservation.rs:213-219`
ya usa. `write_offset` es un upsert sobre la identidad del offset, que es exactamente la
semántica de sobrescritura que el SPI expresa (D-3). Toda consulta liga sus parámetros; ningún
identificador ni valor se interpola en el texto SQL, y toda consulta de offset lleva `tenant`
como parámetro ligado.

Nada más cambia. Los adaptadores heredan las impls genéricas de reenvío `Arc<T>`, satisfacen la
compuerta existente mediante el mecanismo `is_durable()` existente, y se registran mediante la
llamada existente `AppBuilder::read_side_progress`.

## Garantías y Limitaciones Nombradas

**Esta sección es criterio de aceptación, no comentario.** Declara de qué puede depender un host
y, con igual peso, de qué no.

> **Restricción de adopción.** `ego-rs` apunta a producción distribuida, de modo que
> multi-réplica es el objetivo real de despliegue — y esa es exactamente la configuración que
> este cambio no vuelve segura. **PROD-014B es adoptable en Producción solo bajo
> escritor-único-por-`(projection_id, tag, tenant)`, hasta que exista un mecanismo de reserva
> atómica o imposición equivalente (F-1).** Esto es una restricción de adopción declarada, no
> una advertencia: un host que ejecute dos réplicas de la misma proyección queda fuera de la
> garantía, y nada en este cambio lo detecta ni lo rechaza.

### Lo que PROD-014B garantiza

- **G-1** — Los offsets y registros de deduplicación escritos por estos adaptadores sobreviven al
  reinicio del proceso: tras un reinicio, una proyección reanuda desde su último offset
  persistido en lugar de reprocesar todo el flujo sin memoria de deduplicación.
- **G-2** — La tabla de contabilidad de deduplicación converge. Marcar el mismo
  `(projection_id, tag, event_id)` más de una vez — secuencial o concurrentemente — produce
  exactamente una fila y ningún error.
- **G-3** — Una composición `Profile::Production` que registre este par pasa la compuerta de
  PROD-014A por la fuerza de un backend durable real, no de un doble de prueba. Pasar esa
  compuerta significa que el estado de progreso es durable; no significa que el despliegue sea
  seguro ante múltiples escritores (ver la restricción de adopción arriba).
- **G-4** — Los offsets están aislados por `(projection_id, tag, tenant)`; el progreso de un
  tenant nunca es observable como el de otro.

### Lo que PROD-014B NO garantiza

- **L-1 — No es exactamente-una-vez.** PROD-014B entrega procesamiento al-menos-una-vez con
  contabilidad de deduplicación de mejor esfuerzo. Nada en este cambio vuelve exactamente-una-vez
  el manejo de eventos del lado de lectura, y nada en el cambio puede documentarse como si lo
  hiciera.
- **L-2 — No hay deduplicación segura bajo escritores concurrentes multi-nodo reales.** `seen()`
  y `mark_seen()` son métodos de trait separados (`crates/domain/src/read_side/dedup.rs:37-51`) y
  `ReadSideSession::execute` ejecuta el handler de eventos **entre** la verificación
  (`session.rs:116-128`) y el commit (`session.rs:142-149`). Dos escritores sobre el mismo
  `(projection_id, tag, tenant)` pueden observar ambos `seen() == false` y **ambos ya haber
  ejecutado el handler** antes de que alguno marque. La restricción UNIQUE (G-2) corrige la
  contabilidad — la tabla converge, sin fila duplicada, sin error — y no hace nada respecto de la
  doble ejecución. Es una brecha a nivel de SPI; ningún adaptador PostgreSQL puede cerrarla desde
  dentro de `mark_seen`.
- **L-3 — La prevención de doble ejecución del handler depende de una suposición externa y no
  impuesta.** `TagSchedulerImpl::start_projection`
  (`crates/runtime/src/read_side/scheduler.rs:66-108`) espera la sesión de cada tag
  secuencialmente, de modo que el escritor-único-por-tag se cumple **dentro de un proceso hoy**.
  Nada lo impone entre réplicas: el código del lado de lectura no contiene elección de líder, ni
  bloqueo, ni lease, ni token de fencing. Un host que ejecute dos réplicas de la misma proyección
  queda fuera de la garantía, y este cambio ni detecta ni rechaza esa configuración. Dado que
  multi-réplica es el objetivo real de despliegue de `ego-rs`, esta es la restricción de adopción
  vinculante declarada arriba — PROD-014B es adoptable en Producción **solo** bajo
  escritor-único-por-`(projection_id, tag, tenant)` hasta que exista F-1.
- **L-4 — El crecimiento del almacenamiento de deduplicación es ilimitado.** Ni purga, ni TTL, ni
  evicción se entregan aquí (D-4). `projection_dedup` crece **linealmente con la cantidad de
  eventos únicos procesados** por una proyección, de forma monotónica y sin cota superior. Las
  personas operadoras deben observar ese conteo de filas como señal operativa, no descubrirlo
  como incidente.
  **Disparador de escalamiento**: si una proyección en producción alcanza millones de filas en
  una ventana corta, F-2 (retención y evicción) escala a P0/P1 y se planifica de forma
  independiente de este cambio — no espera al ciclo de vida de PROD-014B ni a F-1. La retención
  se excluye aquí porque aún no hay datos reales de volumen para dimensionar un horizonte, y una
  ruta de limpieza cambiaría el ciclo de vida, los índices, la operación y probablemente la API.
- **L-5 — `write_offset` es última-escritura-gana.** Sin CAS, sin garantía de orden, sin
  detección de sobrescritura concurrente (D-3) — esta es la semántica propia del SPI,
  implementada fielmente, no una carencia del adaptador.

## Semántica Requerida

```
Dado un almacén de offsets PostgreSQL y una proyección que escribió el offset N
Cuando el proceso reinicia y se llama read_offset para el mismo
      (projection_id, tag, tenant)
Entonces DEBE devolver N — no None, y no un reproceso desde el inicio.

Dado un almacén de offsets PostgreSQL
Cuando se llama read_offset para un (projection_id, tag, tenant) nunca escrito
Entonces DEBE devolver None, y NO DEBE devolver el offset de otro tenant.

Dado un almacén de deduplicación PostgreSQL
Cuando mark_seen se llama dos veces para el mismo (projection_id, tag, event_id),
      secuencial o concurrentemente
Entonces ambas llamadas DEBEN tener éxito, DEBE existir exactamente una fila, y un
      seen() posterior DEBE devolver true.

Dado un almacén de deduplicación PostgreSQL
Cuando el mismo event_id se marca bajo dos tenants distintos para el mismo
      (projection_id, tag)
Entonces DEBE tratarse como ya visto — el tenant no forma parte de la identidad de
      deduplicación.

Dados dos escritores concurrentes sobre el mismo (projection_id, tag, tenant)
Cuando ambos observan seen() == false antes de que alguno marque
Entonces el handler PUEDE ejecutarse dos veces. Este cambio NO lo previene, y la
      especificación DEBE declararlo como limitación aceptada y nombrada en lugar de
      insinuar semántica exactamente-una-vez.

Dada una composición que declara Profile::Production
Cuando registra este par PostgreSQL mediante AppBuilder::read_side_progress
Entonces build() DEBE tener éxito sin ningún cambio en la lógica de la compuerta.
```

## Áreas Afectadas

| Área | Impacto | Descripción |
|------|---------|-------------|
| `crates/persistence/src/postgres/` (archivo(s) nuevo(s) de almacén) | Nuevo | `PostgreSQLOffsetStore`, `PostgreSQLDedupStore` (IS-1, IS-2) |
| `crates/persistence/src/postgres/migrations/013+` | Nuevo | Dos tablas con sus identidades UNIQUE, tenant `NOT NULL` en offsets (IS-3, D-1, D-2) |
| `crates/persistence/src/postgres/mod.rs` | Modificado | Reexportación (IS-4) |
| `examples/reference-app/src/read_side/mod.rs:93-117` | Modificado | `ReadSideProgressStores::postgres(pool)` (IS-5) |
| `examples/reference-app/src/main.rs:109-114` | Modificado | `None` → par real de Postgres; comentario PROD-014A F-1 retirado (IS-6) |
| `integration-tests/tests/infrastructure/` | Nuevo | Suite de conformidad vía `isolated_database()` (IS-7, D-8) |
| `crates/domain/src/read_side/{offset,dedup,session,runner}.rs` | Intacto | Sin cambio de SPI (OOS-1) |
| `crates/service-sdk/src/runtime/builder.rs`, `app/mod.rs` | Intacto | Sin cambio de compuerta ni de registro (OOS-1) |
| `crates/runtime/src/read_side/scheduler.rs` | Intacto | Sin cambio de concurrencia ni de propiedad (OOS-2) |
| `openspec/specs/{read-side-durable-progress,read-side}/spec.md` | Nuevo / Modificado | Deltas según IS-9 |

## Riesgos

| ID | Riesgo | Probabilidad | Mitigación |
|----|--------|--------------|------------|
| R-1 | "Lado de lectura durable en PostgreSQL" se lee después como "la concurrencia del lado de lectura está resuelta", y un host despliega múltiples réplicas de una proyección creyendo que la deduplicación lo protege | Alta | Ese es todo el propósito de **Garantías y Limitaciones Nombradas**: L-1/L-2/L-3 son criterios de aceptación con su propio criterio de éxito (SC-8), declarados en la especificación y en la documentación pública de los adaptadores, no en un mensaje de commit. F-1 se nombra como el seguimiento atómico distinto |
| R-2 | El crecimiento ilimitado de `projection_dedup` se convierte en un incidente operativo | Media | Declarado de frente (D-4, L-4) en lugar de omitido, con F-2 nombrado **y con un disparador de escalamiento escrito**: millones de filas por proyección en una ventana corta escalan F-2 a P0/P1 de forma independiente de este cambio. El conteo de filas es una señal operativa a observar, no una sorpresa. El propio precedente `effect_dedup` del workspace muestra que la retención siempre tiene propiedad separada |
| R-3 | La suite de conformidad prueba la ida y vuelta pero nunca la supervivencia al reinicio — la única propiedad que distingue este cambio del par en memoria | Media | IS-7 nombra la supervivencia al reinicio como caso obligatorio; SC-1 lo afirma explícitamente |
| R-4 | La migración `013+` colisiona con otro cambio en vuelo que toque la misma secuencia | Baja | La secuencia plana tiene propietario único y se re-verifica en apply; la secuencia independiente de `crates/effect-store` no se ve afectada (D-5) |
| R-5 | Presupuesto de revisión: dos adaptadores, migraciones, recableado de la reference-app y una suite de integración pueden superar 400 líneas cambiadas | Media | **Resuelto a favor del alcance, no de dividirlo.** IS-5/IS-6 son Definición de Hecho: sin el cableado de referencia, este cambio entrega infraestructura que ninguna ruta de composición demuestra utilizable. Si el pronóstico supera el presupuesto, la resolución es elevar el presupuesto para este cambio o recortar código accesorio — nunca mover el cableado funcional a una especificación posterior. `sdd-tasks` lo pronostica bajo esa restricción |
| R-6 | El tenant `NOT NULL` (D-2) se descubre después demasiado estricto si alguna vez aparece un concepto de tenant global en el lado de lectura | Baja | Hoy no puede expresarse: `tenant: &str` no admite nulo. Relajar una columna a nulable es una migración hacia adelante; la inversa no lo sería |

## Seguimientos Nombrados (deliberadamente no absorbidos)

- **F-1 — PROD-014C — Reclamo Atómico de Eventos del Lado de Lectura.** Cerrar la brecha L-2/L-3
  para que la doble ejecución del handler se prevenga en lugar de ser meramente improbable, y
  levantar la restricción de adopción de escritor único. El nombre es deliberado: el problema
  real no es persistir la contabilidad de deduplicación — este cambio ya lo hace de forma
  durable — sino **obtener exclusión antes de que el handler se ejecute**. Un escritor debe
  reclamar el evento, no registrar después que procesó uno. La forma que tomaría una solución
  real ya está probada en este workspace: `EffectDedupStore::reserve`
  (`crates/effect-store/src/postgres/mod.rs:699-756`) es **un** único
  `INSERT ... ON CONFLICT DO NOTHING` atómico que reserva **antes** de que corra cualquier efecto
  colateral. Hay dos rutas abiertas — imposición de escritor-único-por-tag a nivel de
  orquestación, o un futuro método SPI atómico de reclamo/reserva — y elegir entre ellas es
  trabajo de ese cambio, no de este. La detección de pares/réplicas y su imposición también
  pertenecen aquí (OOS-2). Este seguimiento debe existir para que "durable en PostgreSQL" nunca
  se confunda después con "corrección de concurrencia del lado de lectura".
  *Nota de identificador*: `explore.md` §Alcance usó especulativamente "PROD-014C" para un posible
  segundo backend (Stoolap). Ese identificador queda reclamado aquí para el reclamo atómico de
  eventos; un segundo backend, si alguna vez se desea, toma un identificador distinto.
- **F-2 — Retención y evicción de deduplicación del lado de lectura.** Un horizonte, una ruta de
  purga y la regla que ate la eliminación de deduplicación al avance del offset (D-4, L-4). La
  ruta de limpieza separada de `crates/effect-store` es el precedente. **Disparador de
  escalamiento**: millones de filas por proyección en una ventana corta de producción elevan esto
  a P0/P1, planificado de forma independiente de PROD-014B y de F-1.
- **F-3 — Un `ReadSideStore` durable.** Heredado sin cambios de PROD-014A (OOS-5); sigue abierto,
  sigue separado.

## Plan de Reversión

Aditivo. Revertir consiste en: eliminar los dos archivos de adaptador y su reexportación,
eliminar las migraciones `013+`, eliminar la suite de conformidad, quitar
`ReadSideProgressStores::postgres` y restaurar `main.rs` a `None` para el progreso del lado de
lectura. Ningún sitio de llamada existente se ve tocado ni por el cambio ni por la reversión, y
ninguna firma de SPI, compuerta o registro cambia en ninguna dirección.

Las dos tablas nuevas son aditivas y nada más las referencia — una reversión puede eliminarlas o
dejarlas sin daño. Al ser nuevas, no existe dato escrito antes de este cambio que migrar o
perder; el estado escrito por los adaptadores entre el despliegue y la reversión se descarta, lo
que degrada una proyección revertida al comportamiento actual de reproceso desde cero en lugar de
corromper nada.

## Dependencias

- PROD-014A (archivado) — los métodos SPI `is_durable()`, la compuerta `Profile::Production` del
  lado de lectura y `AppBuilder::read_side_progress`. Todos consumidos sin cambios.
- PROD-013 (archivado) — `require_durably_configured` y la compuerta que estableció. No se tocan.
- El runner de migraciones existente de `crates/persistence` y sus convenciones de `PgPool`.
- `ego_integration_tests::isolated_database()` para la suite de conformidad.
- Ninguna dependencia externa, crate, servicio o infraestructura nueva. `sqlx` y PostgreSQL ya son
  dependencias del workspace.

## Criterios de Éxito

- [ ] **SC-1** — Tras un reinicio, una proyección que use el par PostgreSQL reanuda desde su
      último offset persistido. Una prueba lo demuestra descartando y reconstruyendo el almacén
      contra la misma base de datos, no afirmando sobre un valor en proceso.
- [ ] **SC-2** — `read_offset` para un `(projection_id, tag, tenant)` no escrito devuelve `None`,
      y nunca el offset de otro tenant.
- [ ] **SC-3** — Marcar el mismo `(projection_id, tag, event_id)` dos veces tiene éxito ambas
      veces, deja exactamente una fila, y deja `seen()` devolviendo `true`.
- [ ] **SC-4** — La identidad de deduplicación es independiente del tenant: el mismo `event_id`
      bajo un tenant distinto para el mismo `(projection_id, tag)` se reporta como ya visto.
- [ ] **SC-5** — Ambos adaptadores reportan `is_durable() == true`, y una composición
      `Profile::Production` que los registre construye correctamente sin cambio alguno en la
      lógica de la compuerta.
- [ ] **SC-6** — La ruta de producción de `examples/reference-app` registra el par real de
      Postgres; el marcador `None` de "PROD-014A F-1" desaparece.
- [ ] **SC-7** — Toda sentencia SQL liga sus parámetros; ningún valor ni identificador se
      interpola en el texto SQL, y toda consulta de offset lleva `tenant` como parámetro ligado.
- [ ] **SC-8** — L-1, L-2, L-3 y L-4 aparecen como limitación explícita y nombrada en la
      especificación **y** en la documentación pública de los adaptadores, en prosa legible por
      una persona. En ninguna parte del cambio entregado se describe este par como
      exactamente-una-vez, seguro ante concurrencia, o apto para escritores de proyección
      multi-réplica.
- [ ] **SC-9** — PROD-014C — Reclamo Atómico de Eventos del Lado de Lectura (F-1) queda registrado
      como seguimiento atómico distinto, referenciando `EffectDedupStore::reserve` como la forma
      probada, de modo que la brecha tenga propietario nombrado y no implícito.
- [ ] **SC-10** — No existe prueba contra PostgreSQL real fuera de `integration-tests/`, y toda
      prueba nueva obtiene su base de datos de `isolated_database()`.
- [ ] **SC-11** — `crates/domain/src/read_side/`, la compuerta y el registro de
      `crates/service-sdk`, y `crates/runtime/src/read_side/scheduler.rs` quedan sin modificar;
      `cargo test --workspace` muestra cero fallos nuevos.
- [ ] **SC-12** — El rustdoc de los adaptadores, el README de persistencia y la documentación de
      configuración declaran la restricción de adopción de
      escritor-único-por-`(projection_id, tag, tenant)`, y ninguno presenta una configuración de
      proyección multi-réplica como oficialmente soportada. El código no lo impone; la
      documentación lo hace legible para una persona operadora, y la detección/imposición queda
      explícitamente diferida a F-1.
