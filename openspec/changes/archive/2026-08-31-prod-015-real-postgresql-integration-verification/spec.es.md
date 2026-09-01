# Specs Delta: PROD-015 — Verificación de Integración contra PostgreSQL Real

> Documento acompañante para revisión. La fuente de verdad canónica es `spec.md` (identificadores
> de requisito y escenarios 1:1).
> Un solo archivo que cubre tres capabilities, según la sección de Capacidades de este cambio:
> una nueva (`real-infrastructure-verification`) y dos modificadas (`event-store`,
> `idempotent-command-processing`).

## Capability: `real-infrastructure-verification` (NUEVA)

### Propósito

Qué invariantes DEBEN demostrarse contra PostgreSQL real, el contrato de admisión que mantiene
chica la suite, y el presupuesto de reloj. Esta capability gobierna la *metodología* de
verificación; las obligaciones de comportamiento de los adaptadores durables en sí se enuncian
en los deltas de `event-store` e `idempotent-command-processing` más abajo.

## Requisitos

### Requisito: Los Adaptadores Durables Quedan Probados Contra los Harnesses de Conformidad Existentes, Textualmente

La suite de verificación DEBE ejecutar `assert_event_store_conformance`
(`crates/testkit/src/event_store.rs:69`) contra `PostgreSQLEventStore` y
`assert_reservation_store_conformance` (`crates/testkit/src/reservation_conformance.rs:963`)
contra `PostgresOperationReservationStore`, usando exactamente las mismas definiciones de
aserción de `ego-testkit` que usan los llamadores en memoria — nunca una copia paralela ni
re-derivada. La atomicidad de la unidad de trabajo (soltar sin commit no persiste nada; una
unidad de trabajo abierta es invisible para un lector concurrente,
`crates/testkit/src/event_store.rs:328-375`) DEBE demostrarse a través de esta misma corrida
de conformidad contra la conexión distinta del pool de `PostgreSQLEventStore`, no mediante un
test propio y separado, salvo que se demuestre que esa propiedad de conexión distinta es falsa
— en cuyo caso se justifica un requisito de seguimiento dedicado, no asumido aquí.

#### Escenario: Los adaptadores durables pasan exactamente las mismas aserciones que los adaptadores en memoria
- DADO `PostgreSQLEventStore` y `PostgresOperationReservationStore`
- CUANDO cada uno se ejecuta a través de su respectivo harness de conformidad
- ENTONCES toda aserción que las implementaciones en memoria ya satisfacen también pasa contra
  el adaptador durable, sin ningún conjunto de aserciones re-derivado o debilitado

#### Escenario: La atomicidad de la unidad de trabajo se demuestra con la corrida de conformidad, no con un test nuevo
- DADA la corrida de conformidad contra `PostgreSQLEventStore`
- CUANDO un append en stage sin commit se lee desde una segunda conexión del pool
- ENTONCES es invisible, y una unidad de trabajo soltada sin commit no persiste nada — ambas
  cosas observadas como parte de la propia corrida de conformidad, sin ningún archivo de test
  adicional creado para este invariante

### Requisito: El Backfill de la Migración 007 Queda Probado Como Transaccional

`aggregate_type_backfill.rs` (`crates/persistence/src/postgres/aggregate_type_backfill.rs`)
DEBE ejercitarse contra una base de datos PostgreSQL real y migrada, y DEBE demostrar: que
abortar antes de su primer `UPDATE` deja la tabla idéntica byte a byte; que una corrida sobre
cero filas elegibles commitea sin efectos secundarios; y que una reversión reincorpora
exactamente el estado que precedía al backfill.

#### Escenario: Abortar antes del primer UPDATE deja la tabla intacta
- DADA una base de datos migrada con filas elegibles para el backfill
- CUANDO la transacción de backfill aborta antes de que se ejecute su primer `UPDATE`
- ENTONCES la tabla es idéntica byte a byte a su estado previo al backfill

#### Escenario: Un rollback explícito tras un UPDATE completado deja la tabla intacta
- DADA una base de datos migrada con filas elegibles para el backfill, y una transacción de
  backfill que ya ejecutó al menos un `UPDATE`
- CUANDO la transacción se revierte explícitamente en vez de commitear
- ENTONCES la tabla es idéntica byte a byte a su estado previo al backfill, probando que es el
  rollback — no solo el orden de las sentencias — lo que garantiza que no hay efecto parcial

#### Escenario: Una corrida de cero filas commitea limpiamente
- DADA una base de datos migrada sin filas elegibles para el backfill
- CUANDO el backfill corre hasta completarse
- ENTONCES la transacción commitea sin filas modificadas y sin error

#### Escenario: Una reversión reincorpora exactamente el estado previo
- DADO un backfill completado
- CUANDO corre su ruta de reversión
- ENTONCES el estado de la base de datos es idéntico al estado inmediatamente anterior a que
  corriera el backfill

### Requisito: Todo Test Nuevo de Verificación Declara Su Propia Admisión, o No Se Admite

Cada archivo de test agregado bajo este esfuerzo de verificación DEBE declarar, en su propio
doc comment, el invariante exacto que demuestra y por qué ese invariante no puede demostrarse
en proceso, por contrato, por conformidad o en tiempo de compilación. Cada test así DEBE
reflejarse de forma consistente en su registro de módulo y en su entrada de ledger rastreada,
sin ninguna deriva entre ambos y el árbol. Dado que el presupuesto end-to-end de la suite ya
está completamente gastado, ningún test agregado bajo esta capability PUEDE archivarse como un
escenario end-to-end nuevo; cada uno DEBE archivarse bajo una categoría no end-to-end y
declarar su propio riesgo de infraestructura.

#### Escenario: Un test sin justificación declarada no se admite
- DADO un archivo de test candidato sin doc comment que declare su invariante y su
  justificación de por-qué-no-en-proceso
- CUANDO corre la verificación de admisión de la suite
- ENTONCES el test no se admite

#### Escenario: El registro y el seguimiento del ledger nunca derivan respecto del árbol
- DADO un archivo de test nuevo agregado a la suite
- CUANDO corre la verificación de consistencia de la suite
- ENTONCES tanto el registro de módulo del archivo como su entrada de ledger existen y
  concuerdan con lo que hay en disco — una discrepancia en cualquier dirección hace fallar la
  verificación

#### Escenario: No se crea ningún escenario end-to-end nuevo
- DADO que el presupuesto end-to-end ya está completamente gastado
- CUANDO se agrega un test nuevo bajo este esfuerzo de verificación
- ENTONCES se archiva bajo una categoría no end-to-end con su propio riesgo de infraestructura
  declarado, nunca como un quinto escenario end-to-end

### Requisito: Los Invariantes de Mayor Criticidad Se Prueban Por Mutación, No Solo Con un Test en Verde

Para los dos invariantes de mayor criticidad — la carrera de fencing con muchos contendientes
y la atomicidad transaccional/de unidad de trabajo de la migración 007 — la suite de
verificación DEBE demostrar que neutralizar el mecanismo bajo prueba hace que el test nuevo
correspondiente falle, mientras la suite preexistente permanece en verde.

#### Escenario: Neutralizar el mecanismo de fencing hace fallar el test de fencing
- DADOS el test de fencing con muchos contendientes y el test preexistente de fencing con un
  solo contendiente (`fencing_window_postgres.rs`), ambos pasando contra el mecanismo real
- CUANDO se neutraliza el mecanismo de fencing
- ENTONCES fallan tanto el test de fencing con muchos contendientes como el test preexistente
  de fencing con un solo contendiente, porque ambos comparten el mismo predicado portante,
  mientras todo test que no ejercite ese predicado permanece sin afectar

#### Escenario: Neutralizar la atomicidad transaccional/de unidad de trabajo hace fallar el test correspondiente
- DADOS los tests de la migración 007 y de atomicidad de unidad de trabajo pasando contra el
  mecanismo real
- CUANDO se neutraliza ese mecanismo de atomicidad
- ENTONCES el test correspondiente falla, y el resto de la suite preexistente permanece en
  verde

### Requisito: La Verificación de Compatibilidad de Versión de PostgreSQL Es un Slice Acotado, Nunca una Segunda Corrida Completa

PG14 DEBE seguir siendo un piso de compatibilidad real y soportado, y verificado. Solo los
invariantes sensibles a versión — el backfill de la migración 007 y cualquier feature de
SQL/catálogo genuinamente capaz de divergir entre versiones de PostgreSQL — DEBEN probarse
contra PG14, a través de un slice separado y acotado. Los invariantes de contención, fencing,
unidad de trabajo y concurrencia de la suite principal (cubiertos arriba y en los deltas de
`event-store` e `idempotent-command-processing` más abajo) DEBEN seguir corriendo únicamente
contra PG16. Esta capability NO DEBE satisfacerse re-corriendo la suite principal una segunda
vez contra PG14.

#### Escenario: El slice de PG14 cubre solo invariantes sensibles a versión
- DADO el slice de compatibilidad con PG14
- CUANDO se enumera su conjunto de tests
- ENTONCES todo test en él apunta a un invariante sensible a versión nombrado (la migración
  007, o una feature de SQL/catálogo nombrada que podría divergir genuinamente) — ningún test
  de contención, fencing o unidad de trabajo aparece en él

#### Escenario: La suite principal nunca se duplica contra PG14
- DADA la suite principal completa ya pasando contra PG16
- CUANDO se evalúa la completitud del slice de PG14
- ENTONCES es un conjunto de tests pequeño y distinto, nunca una segunda ejecución de los
  tests de contención, fencing o unidad de trabajo de la suite principal contra PG14

## Capability: `event-store` (MODIFICADA)

## Requisitos AGREGADOS

### Requisito: La Conformidad del Event Store Se Extiende a los Adaptadores Durables

`PostgreSQLEventStore` DEBE satisfacer exactamente las mismas definiciones de
`assert_event_store_conformance` que gobiernan la implementación en memoria. Pasar la suite de
conformidad en memoria por sí sola NO DEBE tratarse como evidencia suficiente de que una
implementación durable es conforme.

#### Escenario: Un event store durable que falla la conformidad es no conforme
- DADO `PostgreSQLEventStore` ejecutado a través de `assert_event_store_conformance`
- CUANDO cualquier aserción de ese harness falla
- ENTONCES `PostgreSQLEventStore` no es conforme con esta capability, sin importar el estado de
  la implementación en memoria

### Requisito: La Identidad de Stream con Tenant NULL Honra la Comparación de Tres Valores de SQL de Forma Conductual

Las comparaciones de identidad de stream que involucran un tenant `NULL`/systemwide
(`Option::None`) DEBEN verificarse de forma conductual contra PostgreSQL real, no solo
asegurarse desde el esquema/catálogo. La comparación de igualdad ordinaria bajo lógica de tres
valores (`NULL = NULL` no es verdadero) NO DEBE causar que un stream de tenant systemwide sea
omitido en silencio, ni fusionado en silencio con otro stream de tenant systemwide, durante la
resolución de identidad.

#### Escenario: Dos streams distintos de tenant systemwide se resuelven independientemente
- DADOS dos eventos almacenados bajo agregados distintos, ambos con tenant `Option::None`
- CUANDO se resuelve la identidad de cada stream
- ENTONCES cada uno se resuelve independientemente a su propio stream, sin ninguna colisión
  falsa ni omisión falsa causada por el comportamiento de comparación de tres valores de NULL

## Requisitos MODIFICADOS

### Requisito: Unicidad Efectiva Sobre la Identidad del Stream de Eventos

El event store DEBE rechazar un segundo evento escrito para la misma tupla
`(tenant_id, aggregate_type, aggregate_id, version)` — incluso cuando `tenant_id`
representa al tenant NULL/systemwide. Un duplicado DEBE ser rechazado por el
propio store, no meramente por disciplina a nivel de aplicación. Contra una
población real de escritores concurrentes, un duplicado rechazado DEBE manifestarse como un
conflicto que reporta la **versión actual real** del stream, y una carrera de N appends
concurrentes dirigidos a un mismo stream DEBE dejar exactamente un ganador.
(Previamente: solo enunciaba el resultado del rechazo; no enunciaba qué reporta el rechazo
bajo contención concurrente real, ni el resultado de la carrera de N vías.)

#### Escenario: Un duplicado de versión para el mismo agregado con tenant es rechazado
- DADO un evento ya almacenado para `(tenant-a, User, user-7, version=3)`
- CUANDO se agrega un segundo evento para la tupla idéntica
- ENTONCES el store rechaza el segundo append como una violación de unicidad

#### Escenario: Un duplicado de versión bajo el modo systemwide de tenant NULL también es rechazado
- DADO un evento ya almacenado para `(NULL, TenantOrganization, org-1,
  version=1)` en el modo systemwide sin tenant
- CUANDO se agrega un segundo evento para la tupla systemwide idéntica
- ENTONCES el store rechaza el segundo append — la identidad de tenant NULL no
  exime a la tupla de la aplicación de unicidad

#### Escenario: Una carrera de N appends concurrentes deja exactamente un ganador, cada rechazo reportando la versión real
- DADOS N llamadores concurrentes, cada uno agregando el siguiente evento al mismo stream
  idéntico
- CUANDO los N appends se intentan concurrentemente
- ENTONCES exactamente un append tiene éxito, los N-1 restantes se rechazan como conflictos, y
  cada uno de esos N-1 conflictos reporta la versión actual real y ganadora del stream —
  obtenible solo bajo contención concurrente genuina, pasado el punto en que la propia
  transacción del store ya abortó y debe releer el stream por otra conexión, no desde el
  pre-chequeo de versión esperada obsoleta de un único llamador que ya ejercita el arnés de
  conformidad

## Capability: `idempotent-command-processing` (MODIFICADA)

## Requisitos AGREGADOS

### Requisito: La Conformidad del Store de Reservas Se Extiende al Adaptador Durable

`PostgresOperationReservationStore` DEBE satisfacer exactamente las mismas definiciones de
`assert_reservation_store_conformance` que gobiernan a los llamadores existentes del harness.
Pasar esas aserciones solo en un contexto de test no durable NO DEBE tratarse como evidencia
suficiente de que el adaptador durable es conforme.

#### Escenario: Un store de reservas durable que falla la conformidad es no conforme
- DADO `PostgresOperationReservationStore` ejecutado a través de
  `assert_reservation_store_conformance`
- CUANDO cualquier aserción de ese harness falla
- ENTONCES `PostgresOperationReservationStore` no es conforme con esta capability

## Requisitos MODIFICADOS

### Requisito: Lease Con Owner, Expiración y Fencing Verificado

Una reserva en curso DEBE estar gobernada por un lease que porte `owner_id`,
`lease_until` y `fencing_token`. Toda renovación, finalización o abandono de
una reserva DEBE realizar una actualización condicional que verifique
`operation_id + owner_id + fencing_token` en conjunto — almacenar un fencing
token sin verificarlo en cada llamada mutante NO satisface este requisito. Una
actualización presentada por un owner cuyo lease expiró DEBE rechazarse con
`StaleOwner`, y ese owner NO DEBE poder cerrar ni renovar la operación en
adelante. Un llamador posterior DEBE poder tomar el control de un lease
expirado de forma atómica, sacando por fencing al owner anterior. Esto DEBE
sostenerse bajo concurrencia real de múltiples contendientes, no solo en el caso de un solo
contendiente: cuando varios contendientes compiten por un lease expirado, exactamente uno
DEBE ganar, y el fencing token DEBE avanzar exactamente uno — nunca la cantidad de
contendientes.
(Previamente: enunciaba solo la garantía de toma de control con un único contendiente; no
enunciaba el resultado de la carrera con muchos contendientes.)

#### Escenario: La actualización condicional rechaza a un owner obsoleto
- DADA una reserva cuyo lease expiró y fue tomada por un nuevo owner
- CUANDO el owner original intenta completar la reserva
- ENTONCES la actualización condicional falla, se retorna `StaleOwner`, y la
  reserva no es modificada por el llamador obsoleto

#### Escenario: La toma de control atómica saca por fencing al owner anterior
- DADA una reserva con un lease expirado
- CUANDO un nuevo llamador toma el control de la reserva
- ENTONCES la toma de control tiene éxito atómicamente con un nuevo
  `fencing_token`, y cualquier llamada subsiguiente con el fencing token del
  owner anterior falla

#### Escenario: Almacenar un token sin verificarlo es insuficiente
- DADA una implementación que persiste `fencing_token` pero no lo compara al
  renovar/completar/abandonar
- CUANDO un owner obsoleto emite una renovación tras la toma de control
- ENTONCES este requisito NO se satisface — la comparación en la actualización
  condicional es obligatoria, no basta con almacenar el valor

#### Escenario: Seis contendientes compitiendo por un lease expirado dejan exactamente un ganador
- DADA una reserva con un lease expirado y seis contendientes concurrentes intentando la toma
  de control
- CUANDO los seis intentos compiten concurrentemente
- ENTONCES exactamente un contendiente gana la toma de control, y el fencing token avanza
  exactamente uno — no seis

## No-Objetivos

- Pruebas de transición de caída/recuperación de la sonda de readiness (OOS-2) — resiliencia
  del pool de conexiones bajo condiciones de red reales, no una garantía de SQL/transacción/
  fencing de PostgreSQL; la cobertura a nivel unitario ya existe.
- El límite de agotamiento del fencing en `i64::MAX` (OOS-3) — ya cubierto por tests unitarios
  en proceso existentes; PostgreSQL real no agrega nada a ese límite.
- Crear el workspace `integration-tests/`, su runner o su guardia de ledger (OOS-4) — todos ya
  existen y se extienden, no se construyen.
- Verificación de HTTP, socket, OTLP o end-to-end HTTP real de CORE-018 (OOS-1) — no es de
  PostgreSQL, está clasificada como loopback hermético, reservada para una futura PROD-016
  solo a nivel de nombre.
- Arreglar defectos de producción que un test nuevo exponga, más allá de un arreglo pequeño y
  localizado que la fase de diseño acepte explícitamente para IS-4 o IS-2 (OOS-7).
- Docker Compose, en cualquier lugar, para cualquier cosa (OOS-8).
