# Specs Delta: PROD-014B — Stores Durables de PostgreSQL para el Read-Side

> Documento acompañante para revisión. La fuente de verdad canónica es `spec.md` (identificadores
> de requisito y escenarios 1:1).
> Un solo archivo que cubre dos capabilities, según la sección de Capacidades de este cambio:
> una nueva (`read-side-durable-progress`) y una modificada (`read-side`).

## Capability: `read-side-durable-progress` (NUEVA)

### Propósito

El contrato de durabilidad observable para el estado de progreso del read-side respaldado por
PostgreSQL: qué sobrevive a un reinicio de proceso, qué identidad tiene cada registro, qué
retención se promete, y explícitamente qué garantía de concurrencia **no** se ofrece. Esta
capability gobierna únicamente el comportamiento observable del par durable
`OffsetStore`/`DedupStore` — no cubre la interfaz que esos stores implementan, la reclamación
atómica de eventos, ni la retención/eliminación de dedup (ver No-Objetivos).

## Requisitos

### Requisito: El Offset Sobrevive a un Reinicio de Proceso

El sistema DEBE persistir el offset de una proyección para un `(projection_id, tag, tenant)`
dado, de forma que después de un reinicio de proceso, leer ese offset devuelva el último valor
persistido en lugar de un valor ausente o un valor que requiera repetir el stream desde el
principio.

#### Escenario: El reinicio retoma desde el último offset persistido

- DADO que una proyección escribió el offset N para `(projection_id, tag, tenant)` a través
  del par durable
- CUANDO el proceso se reinicia y se lee el offset para el mismo
  `(projection_id, tag, tenant)`
- ENTONCES devuelve N — no ausente, y no una repetición desde el principio

### Requisito: Las Lecturas de Offset Ausente Están Aisladas por Tenant

Leer un offset para un `(projection_id, tag, tenant)` que nunca fue escrito DEBE devolver un
valor ausente, y NUNCA DEBE devolver el offset de otro tenant para el mismo
`(projection_id, tag)`.

#### Escenario: Un offset no escrito devuelve ausente, nunca el valor de otro tenant

- DADO que existen offsets para el tenant A en `(projection_id, tag)` pero nunca se escribieron
  para el tenant B
- CUANDO se lee el offset para el tenant B en el mismo `(projection_id, tag)`
- ENTONCES devuelve ausente — nunca el offset del tenant A

### Requisito: Las Marcas de Dedup Repetidas Convergen a un Solo Registro

Marcar el mismo `(projection_id, tag, event_id)` como visto más de una vez — secuencial o
concurrentemente — DEBE tener éxito en cada llamada, DEBE dejar exactamente un registro de
dedup para esa identidad, y NO DEBE levantar un error en la repetición.

#### Escenario: Una marca duplicada converge sin error

- DADO que `(projection_id, tag, event_id)` ya fue marcado como visto
- CUANDO se marca como visto nuevamente
- ENTONCES la segunda llamada también tiene éxito, existe exactamente un registro para esa
  identidad, y una verificación de "visto" subsiguiente devuelve verdadero

### Requisito: La Identidad de Dedup Es Independiente del Tenant

El mismo `event_id` marcado bajo dos tenants distintos para el mismo `(projection_id, tag)`
DEBE tratarse como una sola identidad — el tenant NO forma parte de la identidad de dedup.

#### Escenario: El event_id idéntico de un segundo tenant ya se reporta como visto

- DADO que `event_id` fue marcado como visto para `(projection_id, tag)` bajo el tenant A
- CUANDO el mismo `event_id` se marca como visto bajo el tenant B para el mismo
  `(projection_id, tag)`
- ENTONCES se reporta como ya visto — la identidad de dedup no varía por tenant

### Requisito: Las Escrituras de Offset Son "Última Escritura Gana"

Una escritura sobre el offset de una proyección DEBE sobrescribir el valor previamente
almacenado, sin compare-and-swap, sin verificación de offset-previo-esperado, y sin detección
de una sobrescritura concurrente. Esto es una implementación fiel del propio contrato de
escritura del offset store, no una limitación del adaptador.

#### Escenario: Una escritura posterior sobrescribe silenciosamente a una anterior

- DADO un offset ya escrito para `(projection_id, tag, tenant)`
- CUANDO se emite una segunda escritura para la misma identidad con un valor distinto, sin
  ninguna coordinación de orden entre ambos escritores
- ENTONCES el valor almacenado pasa a ser el de la última escritura, sin error y sin ninguna
  señal de conflicto para ninguno de los dos escritores

### Requisito: Ambos Stores de Progreso Se Reportan a Sí Mismos Como Durables

El offset store y el dedup store DEBEN reportarse a sí mismos como durables ambos, y una
composición que declara el perfil de durabilidad de producción y registra este par a través
del punto de registro de progreso del read-side existente DEBE construirse exitosamente sin
ningún cambio en la lógica de validación propia de ese perfil.

#### Escenario: Una composición de perfil de producción pasa por durabilidad real

- DADA una composición que declara el perfil de durabilidad de producción
- CUANDO registra este par durable a través del punto de registro de progreso del read-side
  existente
- ENTONCES la composición tiene éxito porque ambos stores se reportan a sí mismos como
  durables, no por un sustituto exclusivo de tests

### Requisito: El Tenant Es Parte Obligatoria de la Identidad del Offset

Cada registro de offset persistido DEBE portar un valor de tenant concreto. El almacenamiento
persistido de offsets NO DEBE aceptar, y NUNCA DEBE contener, un tenant ausente/nulo en un
registro de offset — a diferencia del manejo de tenant anulable/systemwide usado en otras
partes de este framework para los stores del lado de escritura, que no aplica aquí.

#### Escenario: Un registro de offset siempre porta un tenant concreto

- DADO que una proyección escribe un offset
- CUANDO la escritura se persiste
- ENTONCES el campo de tenant del registro almacenado contiene el valor de tenant concreto
  para el que se hizo la escritura — nunca un valor ausente/nulo

### Requisito: El Crecimiento del Almacenamiento de Dedup Es Ilimitado en Esta Capability

Esta capability NO DEBE incluir ningún mecanismo de purga, tiempo de vida (TTL), ni
eliminación para los registros de dedup. El almacenamiento de dedup crece monótonamente con
la cantidad de eventos únicos procesados por una proyección, sin un límite superior, mientras
solo esta capability lo gobierne. Esta es una limitación explícita y nombrada, no una omisión.

#### Escenario: Los registros de dedup se acumulan sin eliminación automática

- DADO que una proyección procesó muchos eventos únicos a lo largo del tiempo
- CUANDO se inspeccionan los registros de dedup de esos eventos
- ENTONCES todos siguen presentes — nada en esta capability purgó, expiró, ni eliminó ninguno
  de ellos

### Requisito: El Camino de Producción de la Aplicación de Referencia Usa el Par Durable

El camino de composición de producción de la aplicación de referencia DEBE registrar un par de
progreso durable y real para su proyección de read-side, en lugar de omitir el progreso del
read-side o sustituirlo por un placeholder no durable.

#### Escenario: La composición de producción ya no omite el progreso del read-side

- DADA la aplicación de referencia componiéndose a sí misma bajo el perfil de durabilidad de
  producción
- CUANDO se alcanza su punto de registro de progreso del read-side
- ENTONCES registra un par durable real — no un valor ausente, y no un placeholder no durable

### Requisito: La Restricción de Adopción de Escritor Único Está Documentada a Nivel de Adaptador

La documentación pública de los adaptadores de esta capability DEBE indicar, en palabras que
un operador pueda leer, que la operación segura depende de escritor-único-por-`(projection_id,
tag, tenant)`, y NO DEBE presentar una configuración de proyección multi-réplica como
oficialmente soportada.

#### Escenario: La documentación del adaptador enuncia la restricción de adopción

- DADO un operador leyendo la documentación pública a nivel de adaptador de esta capability
- CUANDO busca orientación sobre ejecutar réplicas concurrentes de la misma proyección
- ENTONCES la documentación enuncia explícitamente la restricción de adopción de
  escritor-único-por-`(projection_id, tag, tenant)`, y no describe una configuración
  multi-réplica como soportada

## Capability: `read-side` (MODIFICADA)

## Requisitos AGREGADOS

### Requisito: La Contabilidad Durable de Dedup No Implica Ejecución Exactamente-Una-Vez del Handler

Persistir la contabilidad de dedup de forma durable NO DEBE leerse, describirse ni
documentarse en ningún lugar de este sistema como manejo de eventos exactamente-una-vez. Esta
capability entrega ejecución del handler al-menos-una-vez con contabilidad de dedup de mejor
esfuerzo; nada en ella impide que un handler se ejecute más de una vez para el mismo evento
bajo escritores concurrentes.

#### Escenario: Dos escritores concurrentes pueden ejecutar el handler uno cada uno

- DADOS dos escritores procesando el mismo `(projection_id, tag, tenant)` concurrentemente,
  ambos verificando si un evento ya fue visto antes de que cualquiera lo registre como visto
- CUANDO ambos observan el evento como aún-no-visto antes de que cualquiera lo registre
- ENTONCES el handler PUEDE ejecutarse para ambos escritores; esta capability no impide ese
  resultado, y ninguna documentación puede describirlo como impedido

### Requisito: La Prevención de Ejecución Doble del Handler Descansa en una Restricción de Adopción de Escritor Único, Explícita y No Forzada

La prevención de ejecución doble del handler para el mismo evento DEBE enunciarse como
dependiente de una restricción de adopción externa y no forzada —
**escritor-único-por-`(projection_id, tag, tenant)`** — nunca como una garantía que esta
capability misma haga cumplir. No existe elección de líder, lock, lease ni mecanismo de
fencing en esta capability que haga cumplir esa restricción entre múltiples réplicas de la
misma proyección. Esta es la restricción de adopción vinculante del cambio: adoptar este par
durable en producción está condicionado a que se cumpla.

#### Escenario: Un despliegue de dos réplicas queda fuera de la garantía, y sin detectar

- DADAS dos réplicas del mismo proceso de proyección corriendo concurrentemente contra el mismo
  `(projection_id, tag, tenant)`
- CUANDO esta configuración se evalúa contra las garantías de esta capability
- ENTONCES queda fuera de la garantía que ofrece esta capability, y nada en esta capability
  detecta ni rechaza esa configuración

### Requisito: La Brecha de Concurrencia Tiene un Seguimiento Nombrado y Distinto

La brecha entre la contabilidad durable de dedup y la prevención de la ejecución doble del
handler DEBE registrarse como un seguimiento distinto y nombrado — **PROD-014C — Reclamación
Atómica de Eventos del Read-Side** — en lugar de plegarse dentro del alcance de esta
capability o dejarse en silencio sin dueño.

#### Escenario: El seguimiento está nombrado, no implícito

- DADO un lector de la documentación de esta capability buscando cómo se prevendrá
  eventualmente la ejecución doble del handler
- CUANDO busca el seguimiento que lo posee
- ENTONCES lo encuentra nombrado como PROD-014C — Reclamación Atómica de Eventos del
  Read-Side, distinto de esta capability y no parte de ella

## No-Objetivos

- Ningún cambio a la interfaz `OffsetStore`/`DedupStore`, a la lógica de la puerta del perfil
  de durabilidad de producción, ni al mecanismo de registro de progreso del read-side
  existente.
- Ninguna reclamación atómica de eventos, reserva, elección de líder, lock, lease, o token de
  fencing — todos son alcance de PROD-014C — Reclamación Atómica de Eventos del Read-Side, no
  de esta capability.
- Ninguna detección de pares/réplicas de ningún tipo.
- Ninguna retención, TTL, ni mecanismo de eliminación de dedup.
- Ningún backend distinto de PostgreSQL.
- Ninguna eliminación, deprecación, u ocultamiento de los pares de progreso en memoria o
  "fake durable" existentes — permanecen válidos para Dev y tests.
- Ninguna propiedad multi-worker, arrendamiento de particiones, alta disponibilidad, entrega
  exactamente-una-vez, ni orquestación de reconstrucción de proyecciones.
- Ningún cambio a una proyección generada fuera de la raíz de composición.
- Ningún `ReadSideStore` durable (la vista de eventos que una proyección consulta) — un ítem
  distinto, aún abierto.
