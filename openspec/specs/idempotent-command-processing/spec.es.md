# Especificación: Procesamiento Idempotente de Comandos

## Propósito

Define el contrato observable para el procesamiento idempotente de comandos
de extremo a extremo: una `OperationKey` obligatoria suministrada por el
cliente que identifica una operación de negocio completa, una reserva previa
al dispatch con lease/fencing, recibos por agregado confirmados atómicamente
junto con el append de eventos, y dos garantías nombradas por separado, cada
una acotada por un ciclo de vida distinto. Esta especificación fija QUÉ es la
garantía; la forma de los traits, el diseño de tablas y el mecanismo de
unicidad para el tenant NULL son decisiones de la fase de diseño (ver
No-Objetivos).

## Requisitos

### Requisito: Clave Obligatoria en Todo Comando Externo Mutable

Todo comando externo mutable (HTTP hoy; gRPC/Kafka cuando existan esos
transportes) DEBE llevar una `Idempotency-Key` suministrada por el cliente.
Una clave ausente DEBE ser rechazada antes de que la operación sea
despachada. `IdempotencyEnforcementMode` DEBE exponer exactamente una
variante de compatibilidad acotada que permita un período de transición
temporal; su valor por defecto DEBE ser la variante fail-closed (clave
obligatoria).

#### Escenario: Clave ausente rechazada bajo el modo por defecto
- DADO el `IdempotencyEnforcementMode` por defecto
- CUANDO llega un comando mutable sin `Idempotency-Key`
- ENTONCES el comando es rechazado antes del dispatch; ningún agregado es
  tocado

#### Escenario: El modo de compatibilidad es explícito y acotado, nunca silencioso
- DADO `IdempotencyEnforcementMode` configurado en su variante de
  compatibilidad
- CUANDO llega un comando sin clave
- ENTONCES el comando es admitido solo porque esa variante fue configurada
  explícitamente — ningún valor por defecto no documentado permite esto

### Requisito: Sin Generación de Clave del Lado del Servidor

El sistema NO DEBE acuñar una `OperationKey` en nombre del cliente cuando no
existe una. Una clave generada por el servidor es una función de la solicitud
tal como fue recibida y, por lo tanto, no deduplica nada en un reintento.

#### Escenario: El servidor nunca fabrica una clave para una solicitud sin clave
- DADO un comando mutable sin clave suministrada por el cliente
- CUANDO la exigencia está activa
- ENTONCES el sistema rechaza el comando; NO DEBE generar una clave y
  continuar

### Requisito: Identidad Delimitada por Operación, Reservada Antes del Dispatch

Una `OperationKey` DEBE identificar una operación de negocio completa,
potencialmente abarcando múltiples agregados — no una clave por comando de
agregado. La reserva DEBE crearse después de que la evaluación de
`#[authorize]` y `#[tenant_scoped]` tenga éxito, y antes de la primera llamada
a `EntityRuntime`. El espacio de nombres de unicidad de la reserva DEBE ser el
`CanonicalTenant` producido por `TenantResolver::resolve`; NO DEBE estar
delimitado por espacio de nombres según el hint de tenant crudo del cliente.

#### Escenario: Una clave cubre una operación multi-agregado
- DADA una operación que escribe dos agregados bajo una `OperationKey`
- CUANDO la operación se reintenta tras una finalización parcial
- ENTONCES ambos agregados son abordados bajo la misma reserva; no se crea
  una segunda reserva independiente para el segundo agregado

#### Escenario: La reserva ocurre después de la autorización y el alcance de tenant
- DADO un comando que falla `#[authorize]` o `#[tenant_scoped]`
- CUANDO el guard deniega la llamada
- ENTONCES no se crea ninguna reserva — el paso de reserva nunca se ejecuta
  antes de una evaluación de guard exitosa

#### Escenario: El espacio de nombres usa el tenant resuelto, nunca el hint crudo
- DADO un hint de tenant suministrado por el cliente que difiere del
  `CanonicalTenant` resuelto por `TenantResolver::resolve`
- CUANDO la operación es reservada
- ENTONCES la clave de reserva está delimitada por el `CanonicalTenant`
  resuelto, nunca por el hint crudo

### Requisito: Lease Con Owner, Expiración y Fencing Verificado

Una reserva en curso DEBE estar gobernada por un lease que porte `owner_id`,
`lease_until` y `fencing_token`. Toda renovación, finalización o abandono de
una reserva DEBE realizar una actualización condicional que verifique
`operation_id + owner_id + fencing_token` en conjunto — almacenar un fencing
token sin verificarlo en cada llamada mutante NO satisface este requisito.
Una actualización presentada por un owner cuyo lease expiró DEBE ser
rechazada con `StaleOwner`, y ese owner NO DEBE poder cerrar ni renovar la
operación después de eso. Un caller posterior DEBE poder tomar el control de
un lease expirado de forma atómica, dejando fuera de juego (fencing) al owner
anterior. Esto DEBE sostenerse bajo concurrencia real de múltiples contendientes, no solo en
el caso de un solo contendiente: cuando varios contendientes compiten por un lease expirado,
exactamente uno DEBE ganar, y el fencing token DEBE avanzar exactamente uno — nunca la
cantidad de contendientes.
(Previamente: enunciaba solo la garantía de toma de control con un único contendiente; no
enunciaba el resultado de la carrera con muchos contendientes.)

#### Escenario: La actualización condicional rechaza a un owner obsoleto
- DADA una reserva cuyo lease expiró y fue tomada por un nuevo owner
- CUANDO el owner original intenta completar la reserva
- ENTONCES la actualización condicional falla, se retorna `StaleOwner`, y la
  reserva no es modificada por el caller obsoleto

#### Escenario: La toma de control atómica deja fuera al owner anterior
- DADA una reserva con un lease expirado
- CUANDO un nuevo caller toma el control de la reserva
- ENTONCES la toma de control tiene éxito atómicamente con un nuevo
  `fencing_token`, y cualquier llamada posterior con el fencing token del
  owner anterior falla

#### Escenario: Almacenar un token sin verificarlo es insuficiente
- DADA una implementación que persiste `fencing_token` pero no lo compara
  en renew/complete/abandon
- CUANDO un owner obsoleto emite un renew después de una toma de control
- ENTONCES este requisito NO está satisfecho — la comparación en la
  actualización condicional es obligatoria, no basta con almacenar el valor

#### Escenario: Seis contendientes compitiendo por un lease expirado dejan exactamente un ganador
- DADA una reserva con un lease expirado y seis contendientes concurrentes intentando la toma
  de control
- CUANDO los seis intentos compiten concurrentemente
- ENTONCES exactamente un contendiente gana la toma de control, y el fencing token avanza
  exactamente uno — no seis

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

### Requisito: Recibos Por Agregado Confirmados Atómicamente Con el Append

Cada agregado que una operación alcanza DEBE registrar un recibo permanente
con clave `(tenant_id, aggregate_type, aggregate_id, operation_key)`,
confirmado en la misma transacción que el append de eventos de ese agregado.
El recibo DEBE escribirse incluso cuando el comando tiene éxito y no produce
ningún evento. El snapshot o estado en memoria NO DEBE tratarse como la única
fuente de verdad sobre si una operación ya se aplicó a un agregado — el
recibo es la autoridad.

#### Escenario: Un éxito sin eventos igual escribe un recibo
- DADO un comando que tiene éxito sin producir ningún evento
- CUANDO el comando se completa
- ENTONCES se escribe un recibo para ese par agregado/operación dentro de la
  misma transacción que el commit (vacío)

#### Escenario: La confirmación del recibo es atómica con el append
- DADO un comando que produce eventos
- CUANDO la transacción de append hace commit
- ENTONCES el recibo para ese par agregado/operación queda confirmado en la
  transacción idéntica — nunca como una escritura separada y posterior

#### Escenario: El snapshot solo no puede responder "¿ya se aplicó?"
- DADO un agregado cuyo snapshot en memoria avanzó más allá del punto en que
  una operación lo habría afectado
- CUANDO la recuperación pregunta si esa operación ya se aplicó
- ENTONCES la respuesta la determina el recibo persistido, no se infiere del
  estado actual del snapshot

### Requisito: Dos Garantías, Nombradas Por Separado

La ventana de replay y la protección contra duplicación de dominio DEBEN
nombrarse y acotarse por separado. La ventana de replay — la respuesta previa
exacta devuelta en un reintento — está acotada por el TTL de la reserva,
contado desde `completed_at`. La protección contra duplicación de dominio —
ningún agregado vuelve a mutar por una operación que ya se le aplicó — dura
toda la vida del stream, terminando solo con la eliminación explícita y
definitiva del agregado o del tenant. Después de que el TTL expira, no hay
replay de respuesta ni detección a nivel de frontera de una clave reutilizada;
para una operación rechazada antes de tocar cualquier agregado, o exitosa sin
alcanzar uno, la protección termina con el TTL.

#### Escenario: Después del TTL, la respuesta previa ya no se reproduce
- DADA una reserva cuyo TTL transcurrió y fue purgada
- CUANDO se reintenta la misma clave
- ENTONCES no se devuelve ninguna respuesta almacenada — la frontera trata
  esto como una operación nueva a efectos de replay

#### Escenario: Los recibos siguen bloqueando la re-mutación después del TTL
- DADA una reserva purgada tras el TTL, para una operación que ya alcanzó y
  escribió en un agregado
- CUANDO se reintenta la misma clave contra ese agregado
- ENTONCES el recibo permanente del agregado sigue provocando un no-op — el
  agregado no vuelve a mutar, aunque el replay ya no esté disponible

#### Escenario: Una operación de cero agregados pierde toda protección al TTL
- DADA una operación rechazada antes de tocar cualquier agregado, o exitosa
  sin alcanzar uno
- CUANDO su TTL de reserva transcurre
- ENTONCES reutilizar esa clave es indistinguible de una operación nueva —
  este es el límite documentado de la protección, no un defecto

### Requisito: La Huella (Fingerprint) Determina Replay vs. Conflicto

Cada reserva y recibo DEBE registrar una huella del contenido de la operación
junto a su clave. La misma clave con la misma huella DEBE tratarse como ya
aplicada (replay o no-op). La misma clave con una huella distinta DEBE
tratarse como un conflicto permanente — nunca como una deduplicación
silenciosa ni como la reapertura silenciosa de una transacción de negocio.
Esta regla DEBE cumplirse tanto en la frontera de la reserva como en la tabla
de recibos por agregado.

#### Escenario: Misma clave, misma huella reproduce (replay)
- DADA una operación completada bajo la clave K con huella F
- CUANDO K se reintenta con la huella idéntica F
- ENTONCES se devuelve el resultado almacenado (o el agregado hace no-op),
  nunca se re-ejecuta

#### Escenario: Misma clave, huella distinta es un conflicto permanente
- DADA una operación completada o en curso bajo la clave K con huella F
- CUANDO K se reintenta con una huella distinta F'
- ENTONCES la llamada falla con un conflicto permanente; la operación
  original nunca se reabre ni se reinterpreta silenciosamente

#### Escenario: Un desajuste de huella a nivel de recibo también es conflicto
- DADO un agregado que tiene un recibo para la clave K con huella F
- CUANDO un comando posterior presenta K con huella F' durante la recuperación
- ENTONCES la búsqueda del recibo reporta un conflicto, no un no-op

### Requisito: Retención Dividida y Purga Segura

Las reservas y las respuestas almacenadas DEBEN retenerse durante un TTL
configurable contado desde `completed_at`, nunca desde `created_at`. Una
reserva en estado `InProgress` NO DEBE purgarse por TTL; se vuelve elegible
para purga solo después de que su lease expira y se resuelve mediante la ruta
de expiración de lease. Los recibos por agregado DEBEN retenerse
permanentemente durante toda la vida del stream y solo DEBEN poder eliminarse
junto a una eliminación explícita y definitiva del agregado o tenant
propietario — nunca mediante el job de retención ordinario. El job de purga
DEBE ser por lotes, observable y seguro cuando se ejecuta concurrentemente
desde múltiples workers.

#### Escenario: El TTL se mide desde la finalización, no desde la creación
- DADA una reserva creada en T0 y completada en T1
- CUANDO el TTL configurado transcurre desde T1
- ENTONCES la reserva se vuelve elegible para purga en `T1 + TTL`, no en
  `T0 + TTL`

#### Escenario: Las reservas InProgress nunca se purgan por TTL
- DADA una reserva todavía `InProgress` más allá de lo que sería su TTL si
  hubiera completado
- CUANDO se ejecuta el job de purga
- ENTONCES no se purga; solo la expiración de lease y la toma de control
  pueden resolverla

#### Escenario: Los recibos sobreviven la retención ordinaria
- DADOS recibos de un agregado más antiguos que cualquier TTL de reserva
  configurado
- CUANDO se ejecuta el job de purga ordinario
- ENTONCES esos recibos no se eliminan — solo una eliminación explícita de
  agregado/tenant los elimina

#### Escenario: Los workers de purga concurrentes no purgan doble ni bloquean
- DADOS dos workers de purga ejecutándose concurrentemente sobre filas
  elegibles que se superponen
- CUANDO ambos ejecutan un paso de purga
- ENTONCES cada fila elegible se purga exactamente una vez, y ningún worker
  entra en deadlock ni falla a causa del paso concurrente del otro

### Requisito: La Escritura Dual-Agregado No Se Promete Atómica

Esta capacidad NO DEBE prometer atomicidad entre múltiples agregados tocados
por una operación (p. ej., la escritura organización-luego-usuario de
`RegisterUserImpl`). Promete únicamente recuperación segura por
re-ejecución: un agregado que ya tiene un recibo para la operación hace
no-op en el reintento; un agregado que nunca recibió la operación la
ejecuta. Esta capacidad no introduce ningún mecanismo de saga, compensación
ni rollback.

#### Escenario: La finalización parcial se recupera sin duplicación
- DADA una operación cuyo lease expiró después de que se confirmara el
  recibo de un agregado pero antes de alcanzar un segundo agregado
- CUANDO un nuevo owner toma el control y re-ejecuta
- ENTONCES el primer agregado hace no-op sobre su recibo existente, el
  segundo agregado se ejecuta, y la operación se completa con cero eventos
  duplicados — sin ninguna afirmación de atomicidad entre las dos escrituras

### Requisito: OperationKey Es Distinta de IdempotencyKey

`OperationKey` DEBE ser un newtype distinto de la `IdempotencyKey`
existente, definido en el crate de dominio común. NO DEBE existir
`From<OperationKey>` ni ninguna otra conversión implícita entre ambos tipos.
Un puente futuro, si alguna vez se necesita, DEBE ser una función
deliberadamente nombrada, nunca una implementación de trait de conversión
genérica.

#### Escenario: Ninguna conversión implícita compila
- DADOS los tipos `OperationKey` e `IdempotencyKey`
- CUANDO se busca en el workspace una implementación de
  `From<OperationKey> for IdempotencyKey` (o la inversa)
- ENTONCES no existe ninguna — una prueba compile-fail confirma que un
  intento de conversión implícita no compila

#### Escenario: Ambos validan cadenas no vacías pero siguen siendo tipos no relacionados
- DADO un valor válido tanto como `OperationKey` como cadena `IdempotencyKey`
- CUANDO se usa para construir uno de los tipos
- ENTONCES el resultado no puede pasarse a ningún lugar donde se requiera el
  otro tipo sin una función de derivación explícita y nombrada

### Requisito: El Replay Entre Tenants Está Prohibido

Una respuesta almacenada, reserva o recibo con clave bajo el tenant A NUNCA
DEBE reproducirse, devolverse ni tratarse como ya aplicada para una
solicitud resuelta al tenant B — incluyendo cuando el tenant B es el tenant
NULL/systemwide. Este es un requisito de seguridad: el replay entre tenants
es un vector de divulgación de información, no meramente un defecto de
corrección.

#### Escenario: Una clave idéntica entre dos tenants nunca se reproduce entre ellos
- DADO que el tenant A completa una operación bajo la clave K con una
  respuesta almacenada
- CUANDO el tenant B presenta posteriormente la clave idéntica K
- ENTONCES la solicitud del tenant B se evalúa como su propia operación; la
  respuesta almacenada del tenant A nunca se devuelve al tenant B

#### Escenario: Las solicitudes systemwide (tenant NULL) no se filtran hacia ni desde un tenant real
- DADA una operación completada bajo la clave K en el alcance systemwide del
  tenant NULL
- CUANDO un tenant real presenta posteriormente la clave idéntica K
- ENTONCES la solicitud del tenant real se evalúa de forma independiente; la
  respuesta systemwide nunca se reproduce hacia él, y viceversa

### Requisito: La Garantía Es Neutral al Protocolo, Demostrada Por Dos Portadores de Clave

La idempotencia NO DEBE depender de ningún tipo de protocolo. `OperationKey`,
`OperationFingerprint`, la validación de clave y la política de clave faltante
DEBEN vivir en las capas de dominio y SDK y NO DEBEN referenciar ningún tipo
de transporte. Un adaptador de transporte DEBE contribuir únicamente el lugar
del que se lee un valor crudo, nunca una regla sobre él.

Al menos **dos** transportes DEBEN implementar `OperationKeyCarrier` y DEBEN
pasar el mismo arnés de conformidad de tres estados idéntico — un adaptador
puede satisfacer un contrato por accidente, dos no. HTTP y gRPC son el mínimo
requerido para esta capacidad. Todo adaptador DEBE reportar los mismos tres
estados, `Absent`, `Present` y `Unreadable`, y NO DEBE redefinir la
validación ni la exigencia para su propio protocolo.

Este requisito trata sobre la conformidad del portador de clave, no sobre un
segundo transporte de dispatch de comandos funcionando: hoy solo el adaptador
HTTP despacha comandos reales a través de la ruta consciente de idempotencia.
El portador gRPC (`GrpcMetadataCarrier`) implementa `OperationKeyCarrier` y
pasa el arnés compartido para la extracción de metadatos `idempotency-key`,
pero no existe en el workspace ninguna ruta de servicio, socket o dispatch de
comandos gRPC — afirmar "dos transportes funcionando para comandos" sería
falso.

#### Escenario: Dos adaptadores resuelven de forma idéntica para cada clase de entrada
- DADOS un portador HTTP y un portador de metadatos gRPC
- CUANDO cada uno recibe una clave ausente, una clave válida, una clave
  inválida y un valor que no puede leerse como texto
- ENTONCES ambos resuelven al mismo resultado para cada clase, tanto en modo
  fail-closed como en modo de compatibilidad

#### Escenario: La conformidad gRPC es solo de extracción, no una afirmación de dispatch
- DADA la implementación de `OperationKeyCarrier` del portador de metadatos
  gRPC
- CUANDO se cita su resultado en el arnés de conformidad como evidencia
- ENTONCES esto solo establece que su extracción de clave coincide con la de
  HTTP para cada clase de entrada — nunca se lee como evidencia de una ruta
  de dispatch de comandos gRPC funcionando, porque ninguna existe en el
  workspace

#### Escenario: Ningún tipo de protocolo llega al núcleo
- DADAS la capa de dominio, el entity runtime, y las superficies de reserva y
  recibo
- CUANDO se inspeccionan sus superficies públicas e internas
- ENTONCES ningún tipo HTTP o gRPC aparece en ninguna de ellas, y el
  comportamiento de idempotencia es alcanzable sin nombrar un protocolo

#### Escenario: La ruta de extracción a dispatch es compartida, no duplicada
- DADOS dos transportes que cada uno extrae una clave a su manera
- CUANDO una clave resuelta viaja desde `ServiceContext`
- ENTONCES ambos siguen una única ruta idéntica hacia la entidad, de modo
  que agregar un transporte solo añade un paso de extracción y nada más

## No-Objetivos

- Autoridad de activación multi-nodo, membresía, o pruebas de contención
  distribuida (PROD-009).
- Outbox transaccional / publicación atómica de efectos (CORE-030).
- Orquestación de sagas o checkpointing de pasos (CORE-029).
- Revivir `CommandContext.expected_version`.
- Deduplicación de proyección de lectura (`crates/domain/src/read_side/dedup.rs`).
- Prescribir la forma del trait `EventStore`, sync-vs-async, o la ubicación
  física del índice de recibos — ver los No-Objetivos de la especificación
  `event-store`; son decisiones de la fase de diseño.
- Prescribir el mecanismo de unicidad para el tenant NULL (`NULLS NOT
  DISTINCT`, centinela, o índices parciales) — decisión de la fase de
  diseño, restringida únicamente por el requisito "El Replay Entre Tenants
  Está Prohibido" de arriba.
- Exigencia sobre Kafka — el contrato es agnóstico de transporte; un
  adaptador Kafka no existe en el workspace hoy. Los adaptadores HTTP y gRPC
  ya existen y están cubiertos por el requisito de arriba.
