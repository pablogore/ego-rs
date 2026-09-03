# Especificación: Claiming Atómico de Read-Side

> Documento acompañante para revisión. La fuente de verdad canónica es `spec.md` (identificadores 1:1).

## Capacidad: read-side-event-claiming

Propósito: Define únicamente el contrato observable de exclusión: identidad del claim, rechazo de adquisición bajo un claim vigente, renovación de lease, takeover basado en expiración, rechazo de dueño obsoleto mediante fencing, liberación inmediata, preservación del orden, y la puerta de fallo cerrado de Production. La forma exacta del puerto (`try_claim`/`renew`/`complete`/`release` u otra), el mecanismo de almacenamiento (tabla de claim vs. lock de advisory) y el número de migración son decisiones de diseño (ver No-Objetivos).

### Requisito: La Identidad del Claim Es `(projection_id, tag, tenant)`

Un claim DEBE identificarse exactamente por la tripleta `(projection_id,
tag, tenant)` — la misma tripleta que ya posee un offset que avanza
monotónicamente. Un claim sobre un `(projection_id, tag, tenant)` NO DEBE
afectar la adquisición, renovación o liberación de un claim sobre cualquier
otro `(projection_id, tag, tenant)`, incluyendo un `tag` distinto o un
`tenant` distinto del mismo `projection_id`.

#### Escenario: Un claim de un tenant no bloquea el claim de otro tenant

- DADO dos tenants distintos del mismo `projection_id` y `tag`
- CUANDO el stream de un tenant está válidamente reclamado
- ENTONCES el stream del otro tenant permanece reclamable de forma
  independiente, sin verse afectado por el primer claim

### Requisito: La Adquisición Excluye a un Segundo Reclamante Concurrente

Para un `(projection_id, tag, tenant)`, como máximo un worker DEBE
sostener un claim válido a la vez. Cuando dos o más workers intentan
adquirir un claim para la misma identidad al mismo tiempo, exactamente uno
DEBE tener éxito; todos los demás DEBEN ser rechazados. Un worker
rechazado NO DEBE llamar a `fetch` ni invocar el handler para ese stream en
ese ciclo.

#### Escenario: Uno de dos adquirientes concurrentes gana, el otro es rechazado

- DADO dos workers consultando el mismo `(projection_id, tag, tenant)`
- CUANDO ambos intentan adquirir el claim al mismo tiempo
- ENTONCES exactamente uno lo obtiene; el otro es rechazado y no llama a
  `fetch` ni invoca el handler para ese stream en ese ciclo

### Requisito: Un Claim Válido Puede Renovarse Para Extender el Procesamiento

Un worker que sostiene un claim válido DEBE poder extender su lease antes
de que expire, sin perder el claim ni interrumpir un batch en curso.
Mientras un lease permanezca válido — original o renovado — ningún otro
worker PUEDE tomar el stream.

#### Escenario: La renovación durante un batch largo evita el takeover

- DADO un worker que sostiene un claim válido sobre un stream, aún
  procesando un batch largo mientras su lease se acerca a la expiración
- CUANDO renueva el lease
- ENTONCES sigue sosteniendo el claim, y ningún otro worker toma el stream
  mientras el lease renovado permanezca válido

### Requisito: Un Lease Expirado Habilita el Takeover Sin Acción del Operador

Cuando un worker que sostiene un claim se detiene — crashea, es matado, o
se pausa indefinidamente — sin liberarlo, el lease del claim DEBE expirar
eventualmente. Una vez expirado, otro worker DEBE poder tomar el stream sin
intervención del operador y sin esperar indefinidamente, de modo que un
worker muerto no pueda bloquear un stream para siempre.

#### Escenario: El claim de un worker muerto se toma automáticamente

- DADO un worker que adquirió un claim y luego se detuvo sin liberarlo
- CUANDO su lease expira
- ENTONCES otro worker toma el stream sin intervención del operador y sin
  esperar indefinidamente

### Requisito: El Takeover Excluye al Dueño Obsoleto Mediante Fencing

Cada claim DEBE ir acompañado de una prueba de propiedad, y cada escritura
realizada bajo un claim (escritura de offset, escritura de dedup) DEBE
verificar esa prueba antes de aplicarse. Un worker cuyo claim fue tomado
después de que su lease expiró DEBE ver rechazada cualquier escritura
posterior que intente como dueño, y ese rechazo DEBE dejar el estado
almacenado sin modificar — en particular NO DEBE retroceder un offset que
el nuevo dueño ya avanzó.

#### Escenario: La escritura de un dueño obsoleto es rechazada y deja el estado sin modificar

- DADO un worker cuyo claim fue tomado por otro worker después de que su
  lease expiró
- CUANDO ese primer worker se reanuda e intenta escribir estado de offset o
  dedup como dueño
- ENTONCES la escritura es rechazada como dueño obsoleto y deja el estado
  almacenado sin modificar, incluyendo no retroceder un offset que el
  nuevo dueño ya avanzó

### Requisito: La Liberación Normal Hace el Stream Reclamable de Inmediato

Un worker que sostiene un claim válido, que termina su batch y libera el
claim de forma normal, DEBE hacer el stream reclamable de inmediato — la
liberación NO DEBE requerir esperar a que el lease expire.

#### Escenario: Un claim liberado es reclamable de inmediato

- DADO un worker que sostiene un claim válido
- CUANDO termina su batch y libera el claim de forma normal
- ENTONCES el stream se vuelve reclamable de inmediato, sin esperar a que
  el lease expire

### Requisito: El Claiming Preserva el Orden Existente por Stream

El claiming NO DEBE reordenar, intercalar ni saltar eventos dentro de un
stream. Mientras un claim esté sostenido, los eventos DEBEN seguir siendo
manejados en orden de versión ascendente por `(tenant, tag)`, exactamente
igual que antes de que existiera esta capacidad.

#### Escenario: El orden no cambia bajo un claim activo

- DADO un stream cuyo claim sostiene un worker
- CUANDO ese worker procesa un batch
- ENTONCES los eventos se manejan en orden de versión ascendente por
  `(tenant, tag)`, exactamente igual que antes de esta capacidad

### Requisito: La Expiración Se Evalúa de Forma Consistente, Nunca Contra el Reloj Propio de un Worker

La expiración del lease DEBE evaluarse contra una única fuente de tiempo
consistente y determinista compartida en la decisión de takeover — nunca
contra el reloj de pared local de un worker individual leído de forma
independiente. Esto evita que el desfase de reloj entre réplicas cause un
takeover prematuro o tardío.

#### Escenario: La expiración no depende de a qué reloj se le pregunte

- DADO dos workers con relojes locales que se desfasan de forma
  independiente
- CUANDO se toma una decisión de expiración de lease
- ENTONCES la decisión es consistente sin importar qué worker la observe —
  no varía porque el reloj local de un worker marque distinto

### Requisito: `Profile::Production` Falla de Forma Cerrada Sin un Mecanismo de Claim Durable

Una composición que declara `Profile::Production` y registra progreso de
read-side pero ningún mecanismo de claim durable DEBE ser rechazada en
tiempo de composición/bootstrap — nunca diferida al primer poll o al
primer batch — con un error que nombre la capacidad faltante y la llamada
exacta que la corrige. Una composición que declara `Profile::Production` y
registra un mecanismo de claim durable DEBE tener éxito, y el read-side
multi-réplica pasa a estar soportado bajo la restricción operacional
declarada (los efectos del handler siguen siendo al-menos-una-vez — ver el
requisito de frontera más abajo).

#### Escenario: La falta de mecanismo de claim se rechaza en bootstrap, no en el primer poll

- DADO una composición que declara `Profile::Production` y registra
  progreso de read-side pero ningún mecanismo de claim durable
- CUANDO se llama a `build()`
- ENTONCES se rechaza en tiempo de composición/bootstrap, con un error que
  nombra la capacidad faltante y la llamada exacta que la corrige

#### Escenario: Un mecanismo de claim durable registrado permite que el build tenga éxito

- DADO una composición que declara `Profile::Production` y registra un
  mecanismo de claim durable
- CUANDO se llama a `build()`
- ENTONCES tiene éxito, y el read-side multi-réplica pasa a estar
  soportado bajo la restricción operacional declarada

### Requisito: Esta Capacidad Limita el Conteo de Ejecuciones del Handler, Nunca el Conteo de Efectos Externos

El claiming atómico limita cuántas veces el framework invoca el handler
para el batch de un dueño de claim; NO DEBE describirse, documentarse ni
leerse como una garantía sobre un efecto externo que realiza un handler.
Un único worker que sostiene un claim válido para un batch completo y que
crashea después de que el handler tuvo éxito pero antes de que el batch
quede completamente registrado PUEDE tener el handler ejecutado de nuevo
para esos eventos al reanudar — esta capacidad NO lo previene, y ningún
artefacto entregado puede describir esta capacidad como procesamiento
exactamente-una-vez o efectos externos exactamente-una-vez.

#### Escenario: Un crash tras el éxito del handler sigue permitiendo una re-ejecución al reanudar

- DADO un único worker que sostiene un claim válido para el batch completo
- CUANDO crashea después de que el handler tuvo éxito pero antes de que el
  batch quede completamente registrado
- ENTONCES el handler PUEDE ejecutarse de nuevo para esos eventos al
  reanudar; esto no se previene, y ningún artefacto entregado puede
  describirlo como procesamiento exactamente-una-vez o efectos externos
  exactamente-una-vez

### No-Objetivos

- Consenso distribuido, elección de líder global, un coordinador de
  transacciones distribuidas, un reemplazo de grupo de consumidores de
  Kafka, o un rediseño de `EventStore`.
- Efectos externos exactamente-una-vez de cualquier tipo — `Handler<E>`
  permite I/O arbitrario; el claiming solo limita el conteo de
  ejecuciones del handler. Evitar un efecto externo duplicado requiere que
  la propia frontera de efecto del handler lleve el fencing o sea idempotente
  por sí misma.
- Retry/backoff para errores `Transient` — una preocupación adyacente,
  entregable de forma independiente.
- Atomicidad entre tablas para las escrituras de dedup y offset — una
  condición preexistente que esta capacidad ni crea ni cierra.
- Concurrencia intra-proceso entre tags — `TagSchedulerImpl` permanece
  secuencial dentro de un proceso; esta capacidad solo hace segura la
  exclusión entre *procesos* concurrentes.
- Cualquier backend distinto de PostgreSQL.
- Prescribir el conjunto exacto de métodos del puerto, el mecanismo de
  almacenamiento (tabla de claim, row lock, o advisory lock), o el número
  de migración — decisiones de diseño.
