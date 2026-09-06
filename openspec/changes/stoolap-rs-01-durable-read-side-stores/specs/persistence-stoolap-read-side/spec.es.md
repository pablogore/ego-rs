# Especificación: Persistence Stoolap Read-Side

> Compañero en español. Canónico / inglés: `spec.md` (IDs de requisitos y escenarios 1:1).

## Purpose

Existen implementaciones respaldadas por Stoolap de los tres puertos de read-side del framework —
`OffsetStore`, `DedupStore`, `ReadSideClaimStore` — que satisfacen todos los invariantes que ya
define el contrato de cada puerto sin semántica más débil, sobreviven a un cierre y reapertura
limpios del archivo subyacente, se mantienen correctas bajo concurrencia real de múltiples tasks
dentro de un mismo proceso, y se ejercitan mediante una composición real de `Profile::Production`
que el gate existente acepta para stores durables y rechaza para stores volátiles. Esta capacidad
está limitada a un único proceso ego-rs dueño del archivo Stoolap; no existe evidencia de que estos
stores sean seguros cuando un archivo es compartido por múltiples procesos del sistema operativo o
múltiples nodos.

## Requirements

### Requirement: Las Lecturas Y Escrituras De Offset Se Aíslan Por (projection_id, tag, tenant)

`read_offset` MUST devolver `None` para una clave nunca escrita. `write_offset` MUST limitarse
estrictamente a `(projection_id, tag, tenant)`; una escritura en una clave MUST NOT afectar a
ninguna otra clave. `write_offset` MUST ser last-write-wins — esta capacidad no impone
compare-and-swap ni monotonicidad, respetando el contrato del puerto tal como es.

#### Scenario: Una escritura queda aislada a su clave

- GIVEN offsets escritos para dos claves `(projection_id, tag, tenant)` distintas
- WHEN se lee cualquiera de las dos claves
- THEN cada una devuelve solo el offset escrito para esa clave exacta, y una clave nunca escrita
  devuelve `None`

#### Scenario: Una escritura repetida sobrescribe sin imponer orden

- GIVEN un offset ya escrito para una clave
- WHEN `write_offset` se llama de nuevo para la clave idéntica con cualquier valor `Offset`
- THEN el store lo acepta, y `read_offset` devuelve el valor recién escrito

### Requirement: La Identidad De Dedup Es (projection_id, tag, event_id), Sin Tenant, Sin Poda

`seen`/`mark_seen` MUST usar como clave únicamente `(projection_id, tag, event_id)`, sin
parámetro de tenant. `mark_seen` MUST ser idempotente, sin generar error en una repetición. El
store MUST NOT podar, expirar ni desalojar marcas; no existe TTL ni retención en esta capacidad.

#### Scenario: mark_seen es idempotente

- GIVEN un evento ya marcado como visto para un `(projection_id, tag)`
- WHEN `mark_seen` se llama de nuevo para la tripleta idéntica
- THEN la llamada tiene éxito sin error y `seen()` sigue devolviendo `true`

#### Scenario: Ninguna entrada de dedup se poda jamás

- GIVEN una marca de dedup escrita hace un tiempo arbitrariamente largo
- WHEN se consulta `seen()` para ella
- THEN sigue devolviendo `true`

### Requirement: El Otorgamiento Y El Rechazo De Claim Son Mutuamente Excluyentes

`try_claim` MUST devolver `Ok(Some(fence))` exactamente cuando ningún lease vivo (no expirado)
posee `claim_id`, ya sea otorgando en fresco o mediante takeover de un lease vencido, y
`Ok(None)` — un rechazo, no un error — cuando un lease vivo ya lo posee. El `fencing_token` de un
takeover MUST ser estrictamente mayor que cualquiera emitido previamente para ese `claim_id`.

#### Scenario: Un claim vivo rechaza a un segundo aspirante

- GIVEN un claim mantenido con un lease aún no expirado
- WHEN un owner distinto llama a `try_claim` para el `claim_id` idéntico
- THEN devuelve `Ok(None)`, y el fence del holder existente sigue siendo válido

#### Scenario: El takeover de un lease vencido acuña un token estrictamente mayor

- GIVEN un claim cuyo `lease_until` ya pasó
- WHEN un nuevo owner llama a `try_claim` para el `claim_id` idéntico
- THEN devuelve `Ok(Some(fence))` con un `fencing_token` estrictamente mayor que el del holder
  vencido, y el fence del holder vencido ya no verifica

### Requirement: Renew Y Release Verifican El Fence Completo Contra El Estado Vivo, Atómicamente

`renew` y `release` MUST verificar `claim_id` + `owner_id` + `fencing_token` juntos contra la fila
almacenada actual, no contra una lectura previa. Ambos MUST rechazar con
`ClaimError::StaleOwner` un fence que ya no coincide con el claim vivo, y por separado un fence
cuyo lease ya venció, dejando el estado sin modificar en ambos casos. `release` MUST NOT borrar la
fila del claim — MUST fijar un lease ya expirado, manteniendo el fencing token monótono y el
claim inmediatamente reclamable.

#### Scenario: renew rechaza un fence obsoleto o vencido sin mutar el estado

- GIVEN un fence que ya no coincide con el claim vivo, o cuyo lease ya venció
- WHEN se llama a `renew` con él
- THEN falla con `StaleOwner` y el claim almacenado no cambia

#### Scenario: release marca el claim como expirado, no lo borra

- GIVEN un claim sostenido bajo un fence válido
- WHEN se llama a `release` con ese fence
- THEN un `try_claim` posterior para el `claim_id` idéntico tiene éxito de inmediato, y la fila
  del claim sigue existiendo con un lease expirado

### Requirement: Los Tipos De Claim Y La Agotamiento Reutilizan El Puerto De Reservation

`OwnerId` y `FencingToken` usados por este store de claim MUST ser los tipos idénticos que ya
define `crate::operation::reservation` y que usa `OperationReservationStore`, no
redefiniciones paralelas. Que `FencingToken::next()` devuelva `None` MUST manifestarse como
`ClaimError::FencingExhausted`, nunca como un token envuelto o truncado.

#### Scenario: El agotamiento se reporta, no se envuelve

- GIVEN un `claim_id` cuyo fencing token ya está en su valor máximo
- WHEN de otro modo se otorgaría un takeover
- THEN `try_claim` devuelve `ClaimError::FencingExhausted` en lugar de envolver el token

### Requirement: La Expiración Del Lease Siempre La Calcula El Llamador

El store MUST NEVER leer la hora del sistema para decidir si un lease expiró. Toda decisión de
expiración MUST comparar únicamente contra el valor `lease_until` que el llamador suministró en
`try_claim` o `renew`.

#### Scenario: Las decisiones de expiración usan solo el timestamp suministrado

- GIVEN un valor `lease_until` suministrado por el llamador
- WHEN el store decide si un claim sigue vivo
- THEN la decisión depende únicamente de ese valor comparado con el `lease_until` almacenado
  previamente, nunca de una lectura de reloj realizada dentro del store

### Requirement: La Corrección Del Claim Se Mantiene Bajo Concurrencia Real Intraproceso

Cuando varias tasks dentro del mismo proceso llaman a `try_claim` concurrentemente para el
`claim_id` idéntico, exactamente una MUST recibir `Ok(Some(fence))`. Toda otra llamada
concurrente MUST recibir `Ok(None)` o un `ClaimError::Transient` seguro de reintentar — nunca un
segundo `Ok(Some(fence))` para el mismo lease vivo.

#### Scenario: Los aspirantes concurrentes producen exactamente un ganador

- GIVEN varias tasks en el mismo proceso llamando a `try_claim` concurrentemente para el
  `claim_id` idéntico, sin lease vivo existente
- WHEN todas las llamadas terminan
- THEN exactamente una recibe `Ok(Some(fence))` y toda otra recibe `Ok(None)` o un error
  `Transient` seguro de reintentar

### Requirement: El Estado De Offset Y Dedup Sobrevive Al Cierre Y La Reapertura

Un valor escrito a través del store de offset o de dedup MUST permanecer legible y sin cambios
tras cerrar el store y reabrir el mismo archivo Stoolap subyacente. `is_durable()` MUST devolver
`true` para cualquiera de los dos stores solo una vez demostrado esto, y `false` en caso
contrario.

#### Scenario: Un offset sobrevive a un ciclo de cierre/reapertura

- GIVEN un offset escrito para una clave
- WHEN el store se cierra y se reabre el mismo archivo
- THEN `read_offset` para esa clave devuelve el valor idéntico

#### Scenario: Una marca de dedup sobrevive a un ciclo de cierre/reapertura

- GIVEN un evento marcado como visto
- WHEN el store se cierra y se reabre el mismo archivo
- THEN `seen()` para ese evento sigue devolviendo `true`

### Requirement: La Durabilidad Del Claim Es Cierre-Y-Reapertura, No Recuperación Ante Caídas

Donde `is_durable()` reporta `true` para el store de claim, el owner, el fencing token y el
estado del lease de un claim MUST sobrevivir a un cierre limpio y una reapertura del mismo
archivo, y un claim liberado o vencido naturalmente MUST reabrirse como reclamable. Esta
capacidad MUST NOT afirmar protección contra un `kill -9` del proceso o una pérdida de energía —
solo contra un ciclo de cierre/reapertura limpio.

#### Scenario: El fence de un claim sobrevive a un ciclo de cierre/reapertura

- GIVEN un claim sostenido bajo un fence válido, con su lease aún no expirado
- WHEN el store se cierra y se reabre el mismo archivo
- THEN `try_claim` para un owner distinto sobre el `claim_id` idéntico sigue devolviendo
  `Ok(None)`, y el fence sostenido sigue verificando contra el estado reabierto

### Requirement: Una Composición Real De Profile::Production Ejercita El Gate, Con Un Control Negativo

El gate de durabilidad de read-side de `Profile::Production` existente MUST ser ejercitado por
una composición real usando stores reales de offset, dedup y claim respaldados por Stoolap,
configuración real, y el punto de entrada real `try_build()` (o equivalente) — no solo afirmando
`is_durable()` sobre un store aislado. El mismo gate, sin modificar, MUST rechazar una
composición donde cualquiera de los tres stores sea una implementación no durable (volátil).

#### Scenario: Una composición real durable con Stoolap pasa Production

- GIVEN `Profile::Production` configurado con stores reales de offset, dedup y claim
  respaldados por Stoolap sobre una base de datos en disco
- WHEN se construye la composición
- THEN tiene éxito

#### Scenario: Un store volátil es rechazado por el gate sin modificar

- GIVEN la composición idéntica con uno de los stores reemplazado por una implementación no
  durable
- WHEN la composición se construye bajo `Profile::Production`
- THEN es rechazada por el mismo gate, sin modificar

### Requirement: Limitado Únicamente A Concurrencia Intraproceso

Esta capacidad MUST NOT describirse, probarse ni documentarse como segura para acceso
concurrente al mismo archivo Stoolap desde más de un proceso del sistema operativo o más de un
nodo. Toda afirmación de concurrencia MUST estar limitada a tasks dentro de un mismo proceso.

#### Scenario: No se afirma ninguna concurrencia multiproceso ni multinodo

- GIVEN la documentación y la suite de tests de esta capacidad
- WHEN se inspeccionan en busca de una afirmación de concurrencia
- THEN toda afirmación está limitada a concurrencia intraproceso, y ninguna asegura seguridad
  entre múltiples procesos o nodos

## Non-Goals

- `EventStore`, `Snapshot`, `Repository`, `EffectStateStore`, `EffectDedupStore`,
  `OperationReservationStore` (ya entregado; reutilizado únicamente como plantilla del patrón de
  concurrencia), y cualquier otro store.
- Cualquier cambio en PostgreSQL, y cualquier cambio en los contratos de trait `OffsetStore`,
  `DedupStore` o `ReadSideClaimStore`.
- Coordinación multiproceso, multinodo, distribuida o de Kubernetes; `LISTEN`/`NOTIFY`; brokers;
  buses de eventos.
- Poda, TTL o retención de dedup.
- Monotonicidad o compare-and-swap en `write_offset`.
- Lógica específica de Verimand o de cualquier otro producto.
- Garantías de durabilidad ante caídas/pérdida de energía (`kill -9`) — solo está en alcance la
  durabilidad de cierre/reapertura limpios.
