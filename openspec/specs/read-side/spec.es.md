# Spec: Read-Side Progreso — Ciclo de Vida Spawn/Stop y Almacenamiento Durable

> Documento acompañante para revisión. La fuente de verdad canónica es `spec.md` (identificadores
> de requisito y escenarios 1:1).

## Capability: read-side

Propósito: `TagSchedulerImpl` gana una llamada, `spawn_projection`, que cablea
la fontería del ciclo de vida stop/join que toda aplicación que la consume previamente
había mano-arreglado alrededor de `run_until_stopped` — generando el bucle de encuesta y devolviendo
un manejador cuya `stop()` se consume a sí misma, espera a que se drene un lote en vuelo, y
superficializa un drenaje fallido en lugar de tragárselo silenciosamente.

Adicionalmente, esta capability documenta las restricciones críticas en implementaciones de almacenes
de progreso duraderos cuando se usan en producción: la contabilidad de dedup durable no implica
ejecución exactamente-una-vez del handler, y la prevención de doble ejecución del handler descansa
en una restricción de adopción de escritor único, explícita y no forzada. Esta capability especifica
solo el comportamiento observable de la fontería spawn/stop y estas restricciones de adopción; el
motor del scheduler subyacente (`TagSchedulerImpl`, `run_until_stopped`, y amigos) y la interfaz
de `OffsetStore` y `DedupStore` quedan fuera del alcance e inalterados.

**Explícitamente no cubierto por esta capability** (ver No-Objetivos): construir un dedup store,
un offset store, un mecanismo de descubrimiento de tags, un handler, o el propio modelo de lectura
consultable de la aplicación. Todos estos permanecen siendo responsabilidad de la aplicación que la
consume, exactamente como antes de que esta capability existiera — una aplicación que la consume
todavía los arregla ella misma (p. ej. propia `ReadSideHandles` de reference-app, inalterada por
esta capability) y los pasa a `spawn_projection` como argumentos.

### Requisito: División de Propiedad — La Aplicación Posee el Modelo de Lectura, el Constructor Posee Solo el Ciclo de Vida Spawn/Stop

El modelo de lectura consultable (la propia vista de lectura de dominio de la aplicación, p. ej.
un tipo shaped `UsersByTenantStore`) es construido y poseído por la aplicación que la consume,
exactamente como antes de que esta capability existiera — es lógica de dominio de aplicación, no
fontería framework, y esta capability no la envuelve, reemplaza, ni la devuelve. La única
responsabilidad de `spawn_projection` es el ciclo de vida spawn/stop/drenaje de la tarea de fondo:
iniciar el bucle de encuesta y devolver un manejador que después puede pararlo y observar cómo
terminó. El dedup store, offset store, mecanismo de descubrimiento de tags, handler, e intervalo
de encuesta son suministrados por el llamador en el mismo sitio de llamada, no construidos
internamente.

#### Escenario: El resultado de la llamada es un manejador de ciclo de vida, no un modelo de lectura agrupado

- DADO que una aplicación ya ha construido su propio modelo de lectura consultable,
  dedup store, offset store, y cierre de descubrimiento de tags
- CUANDO los pasa todos a `spawn_projection`
- ENTONCES `spawn_projection` devuelve solo un manejador de poller — la propia referencia de modelo
  de lectura de la aplicación es lo que consulta directamente, no algo devuelto o re-envuelto por
  la llamada

### Requisito: Conveniencia del Ciclo de Vida Spawn/Stop

`TagSchedulerImpl` DEBE exponer una llamada que, dado un cierre de descubrimiento de tags,
intervalo de encuesta, handler, event store, dedup store, offset store, reportero de progreso,
y callback de error, genera el bucle de encuesta y devuelve un manejador cubriendo su ciclo de
vida stop/drenaje completo — reemplazando la señalización de stop y rastreo de compleción que un
llamador previamente tenía que mano-arreglarse alrededor de `run_until_stopped`. El intervalo de
encuesta DEBE ser un argumento explícito, obligatorio a esa misma llamada (no hardcodeado, no por
defecto, no configurado a través de un paso setter/builder separado) — un llamador con diferentes
necesidades de intervalo (p. ej. un intervalo de encuesta rápido en tests) suministra su propio
valor en el mismo sitio de llamada.

#### Escenario: Una llamada produce un poller generado con fontería completa de ciclo de vida

- DADO que una aplicación ya tiene construido su dedup store, offset store, cierre de
  descubrimiento de tags, handler, y event store
- CUANDO llama a `spawn_projection` con esos valores e intervalo de encuesta explícito
- ENTONCES el bucle de encuesta se genera y el llamador recibe un solo manejador cubriendo su ciclo
  de vida stop/drenaje, sin ningún rastreo de stop-signaling o completion-tracking separado que el
  llamador tenga que mano-arreglarse

#### Escenario: El intervalo de encuesta es obligatorio, no por defecto

- DADOS dos aplicaciones con diferentes necesidades de intervalo de encuesta (p. ej. cadencia de
  producción vs. un intervalo rápido para tests)
- CUANDO cada una llama a `spawn_projection`
- ENTONCES cada una suministra su propio valor de intervalo en el sitio de llamada; ninguna obtiene
  un default silenciosamente-hardcodeado que no puede sobrescribir

### Requisito: Stop Consume el Manejador

La operación stop del manejador de poller DEBE tomar propiedad del manejador (no una referencia
compartida o exclusiva) — una vez parado, el manejador no puede ser reutilizado o parado de nuevo,
haciendo un double-stop un error de tiempo de compilación en lugar de tiempo de ejecución.

#### Escenario: Un manejador parado no puede ser parado de nuevo

- DADO que un llamador sostiene un manejador de poller
- CUANDO llama a stop en ese manejador
- ENTONCES el manejador es consumido por esa llamada, y ninguna otra operación en ese mismo
  valor de manejador es posible

### Requisito: Descubrimiento Dinámico de Tags Per-Tenant Preservado

`spawn_projection` DEBE llamar al cierre de descubrimiento de tags suministrado por el llamador
fresco en cada encuesta, en lugar de cachear su resultado de la primera llamada — preservando la
garantía de aislamiento per-tenant de CORE-018 (una corriente de tags por tenant) sin regresión.
Esta capability no cambia lo que el cierre descubre o cómo los tags son computados — solo que
`spawn_projection` continúa invocándolo por iteración en lugar de una vez en tiempo de generación.

#### Escenario: El primer evento de un tenant se recoge sin reconfiguración

- DADO un manejador de poller ya generado, sin eventos previos para el tenant `T`
- CUANDO el primer evento para el tenant `T` es escrito al event store
- ENTONCES una encuesta subsiguiente descubre y procesa el tag del tenant `T` sin que el
  poller sea regenerado o explícitamente informado sobre el nuevo tenant

### Requisito: Apagado Elegante Preservado

Detener el poller generado DEBE dejar que cualquier lote de encuesta ya en vuelo termine de
drenar antes de que la llamada stop devuelva, y DEBE superficializar un drenaje fallido al
llamador como un error en lugar de descartarlo silenciosamente.

#### Escenario: Stop espera a que un lote en vuelo termine de drenar

- DADO que el bucle de encuesta del poller está en medio de un lote cuando se solicita stop
- CUANDO el llamador llama a stop
- ENTONCES stop no devuelve hasta que ese lote en vuelo haya terminado de drenar

#### Escenario: Un drenaje fallido es reportado, no tragado

- DADO que la tarea de fondo del bucle de encuesta generado termina anormalmente (pánico
  o es abortada) en lugar de drenar limpiamente
- CUANDO el llamador llama a stop
- ENTONCES stop devuelve un error identificando el fracaso, en lugar de reportar
  éxito de todas formas

### Requisito: Usable Por una Aplicación Real Sin Escotillas de Escape

`spawn_projection` DEBE ser suficiente para que una aplicación real que la consume obtenga el
mismo poller generado que de otra forma mano-cableara alrededor de `run_until_stopped`, sin
necesidad de escotillas de escape específicas de aplicación más allá de suministrar su dedup store,
offset store, cierre de descubrimiento de tags, handler, event store, intervalo de encuesta,
reportero de progreso, y callback de error.

#### Escenario: El glue mano-cableado de una aplicación migra a `spawn_projection`

- DADO que una aplicación previamente mano-rodó su propio stop-signaling y
  completion-tracking alrededor de la llamada `run_until_stopped` del motor del scheduler (mientras
  todavía construía su propio dedup store, offset store, cierre de descubrimiento de tags, y modelo
  de lectura, como lo hace hoy)
- CUANDO llama a `spawn_projection` en cambio, pasando esos mismos valores ya-construidos
- ENTONCES ya no mano-rodó el stop-signaling o completion-tracking ella misma, y el aislamiento de
  tags per-tenant continúa funcionando sin cambios; su construcción de dedup store, offset store,
  descubrimiento de tags, y propiedad de modelo de lectura quedan sin afectar (ver "División de
  Propiedad" arriba)

### Requisito: La Aceptación de Raíz de Composición De Un Par de Progreso Durable Construido por el Host Está En Alcance; La Construcción por Framework Permanece Fuera De Alcance

Un par de progreso durable de una proyección — su `OffsetStore` y `DedupStore`
— PUEDE ser compuesto en la raíz de composición: aceptado, clasificado por durabilidad, y rechazado
allí bajo `Profile::Production` cuando no es durable. Esto es ortogonal a, y no revierte, el
no-objetivo existente de CORE-026 de que el framework construya o por-defecto estos stores
internamente — ese no-objetivo permanece completamente en vigor. La raíz de composición nunca
construye internamente un `OffsetStore`, `DedupStore`, o mecanismo de descubrimiento de tags en
nombre de la aplicación; solo acepta, clasifica, y valida un par que la aplicación ya construyó.

#### Escenario: La raíz de composición clasifica y valida sin construir

- DADA una aplicación que ya ha construido su propio par
  `OffsetStore`/`DedupStore`
- CUANDO registra ese par en la raíz de composición
- ENTONCES la raíz de composición clasifica y valida la durabilidad del par
  sin ella misma construir ninguno de los stores

#### Escenario: Una aplicación que no registra nada queda sin afectar

- DADA una aplicación que nunca registra un par de progreso durable en
  la raíz de composición
- CUANDO ella compone su cableado read-side exactamente como antes de este cambio
- ENTONCES nada sobre ese cableado es requerido o realizado por esta
  capability, inalterado de antes

#### Escenario: El rechazo nunca alcanza el motor del scheduler

- DADO un par registrado rechazado bajo `Profile::Production`
- CUANDO ese rechazo ocurre
- ENTONCES ocurre en la raíz de composición, nunca dentro de
  `ProjectionSpec::new()`, `TagSchedulerImpl::spawn()`, o el primer
  lote de encuesta

### Requisitos AGREGADOS (PROD-014B)

#### Requisito: La Contabilidad Durable de Dedup No Implica Ejecución Exactamente-Una-Vez del Handler

Persistir la contabilidad de dedup de forma durable NO DEBE leerse, describirse ni
documentarse en ningún lugar de este sistema como manejo de eventos exactamente-una-vez. Esta
capability entrega ejecución del handler al-menos-una-vez con contabilidad de dedup de mejor
esfuerzo; nada en ella impide que un handler se ejecute más de una vez para el mismo evento
bajo escritores concurrentes.

##### Escenario: Dos escritores concurrentes pueden ejecutar el handler uno cada uno

- DADOS dos escritores procesando el mismo `(projection_id, tag, tenant)` concurrentemente,
  ambos verificando si un evento ya fue visto antes de que cualquiera lo registre como visto
- CUANDO ambos observan el evento como aún-no-visto antes de que cualquiera lo registre
- ENTONCES el handler PUEDE ejecutarse para ambos escritores; esta capability no impide ese
  resultado, y ninguna documentación puede describirlo como impedido

#### Requisito: La Prevención de Ejecución Doble del Handler Descansa en una Restricción de Adopción de Escritor Único, Explícita y No Forzada

La prevención de ejecución doble del handler para el mismo evento DEBE enunciarse como
dependiente de una restricción de adopción externa y no forzada —
**escritor-único-por-`(projection_id, tag, tenant)`** — nunca como una garantía que esta
capability misma haga cumplir. No existe elección de líder, lock, lease ni mecanismo de
fencing en esta capability que haga cumplir esa restricción entre múltiples réplicas de la
misma proyección. Esta es la restricción de adopción vinculante del cambio: adoptar este par
durable en producción está condicionado a que se cumpla.

##### Escenario: Un despliegue de dos réplicas queda fuera de la garantía, y sin detectar

- DADAS dos réplicas del mismo proceso de proyección corriendo concurrentemente contra el mismo
  `(projection_id, tag, tenant)`
- CUANDO esta configuración se evalúa contra las garantías de esta capability
- ENTONCES queda fuera de la garantía que ofrece esta capability, y nada en esta capability
  detecta ni rechaza esa configuración

#### Requisito: La Brecha de Concurrencia Tiene un Seguimiento Nombrado y Distinto

La brecha entre la contabilidad durable de dedup y la prevención de la ejecución doble del
handler DEBE registrarse como un seguimiento distinto y nombrado — **PROD-014C — Reclamación
Atómica de Eventos del Read-Side** — en lugar de plegarse dentro del alcance de esta
capability o dejarse en silencio sin dueño.

##### Escenario: El seguimiento está nombrado, no implícito

- DADO un lector de la documentación de esta capability buscando cómo se prevendrá
  eventualmente la ejecución doble del handler
- CUANDO busca el seguimiento que lo posee
- ENTONCES lo encuentra nombrado como PROD-014C — Reclamación Atómica de Eventos del
  Read-Side, distinto de esta capability y no parte de ella

### No-Objetivos

- Ningún cambio a `TagSchedulerImpl` o al contrato propio del motor subyacente CORE-005 scheduler/store
  — esta capability especifica solo el comportamiento observable de la fontería spawn/stop construida
  en top de él. Explícitamente inalterado: semántica de encuesta (cómo/cuándo se dispara una encuesta),
  semántica de dedup (qué cuenta como duplicado), semántica de offset (cómo se rastrea el progreso y
  se reanuda), y garantías de ordenamiento (orden de entrega per-tag) — esta capability envuelve el
  contrato existente de ese motor, no renegocia ninguna parte de él.
- Ningún nuevo formato de persistencia ni capacidad de consulta de modelo de lectura más allá de lo que
  ya existe.
- Ningún cambio en qué tipo posee el modelo de lectura consultable — permanece completamente poseído
  por la aplicación (ver "División de Propiedad" arriba); esta capability no introduce un tipo de
  modelo de lectura poseído por el framework.
- **Construir un dedup store, un offset store, o un mecanismo de descubrimiento de tags está fuera de
  alcance.** `spawn_projection` toma estos como argumentos obligatorios; no provee un defecto o los
  construye internamente. Una aplicación los obtiene exactamente como lo hace hoy (p. ej. propia
  `ReadSideHandles::new` de reference-app, inalterada por esta capability) y los pasa a
  `spawn_projection` para generar el poller. Una conveniencia a nivel framework que también construye
  estos internamente (p. ej. por-defecto a implementaciones en memoria) fue considerada y rechazada —
  ver design.md AD-1, alternativa (b) — porque el handler y el cierre de descubrimiento de tags son
  irreduciblemente específicos de aplicación, y agrupar la construcción de dedup/offset stores con
  ellos solo cubre la mitad del boilerplate mientras sugiere que la otra mitad también se resolvió.
- Ningún paso "construct" separado no-generador existe a nivel de esta capability — `spawn_projection`
  siempre genera inmediatamente cuando es llamado. Una aplicación que necesita construir su cableado
  read-side sin un runtime async corriendo (p. ej. para asserción en su propio modelo de lectura en un
  test síncrono) lo hace a través de su propio constructor pre-existente (p. ej. `ReadSideHandles::new`),
  que esta capability no cambia ni reemplaza.

## Capability: read-side-durable-progress (NUEVA)

### Propósito

El contrato de durabilidad observable para el estado de progreso del read-side respaldado por
PostgreSQL: qué sobrevive a un reinicio de proceso, qué identidad tiene cada registro, qué
retención se promete, y explícitamente qué garantía de concurrencia **no** se ofrece. Esta
capability gobierna únicamente el comportamiento observable del par durable
`OffsetStore`/`DedupStore` — no cubre la interfaz que esos stores implementan, la reclamación
atómica de eventos, ni la retención/eliminación de dedup (ver No-Objetivos).

### Requisitos

#### Requisito: El Offset Sobrevive a un Reinicio de Proceso

El sistema DEBE persistir el offset de una proyección para un `(projection_id, tag, tenant)`
dado, de forma que después de un reinicio de proceso, leer ese offset devuelva el último valor
persistido en lugar de un valor ausente o un valor que requiera repetir el stream desde el
principio.

##### Escenario: El reinicio retoma desde el último offset persistido

- DADO que una proyección escribió el offset N para `(projection_id, tag, tenant)` a través
  del par durable
- CUANDO el proceso se reinicia y se lee el offset para el mismo
  `(projection_id, tag, tenant)`
- ENTONCES devuelve N — no ausente, y no una repetición desde el principio

#### Requisito: Las Lecturas de Offset Ausente Están Aisladas por Tenant

Leer un offset para un `(projection_id, tag, tenant)` que nunca fue escrito DEBE devolver un
valor ausente, y NUNCA DEBE devolver el offset de otro tenant para el mismo
`(projection_id, tag)`.

##### Escenario: Un offset no escrito devuelve ausente, nunca el valor de otro tenant

- DADO que existen offsets para el tenant A en `(projection_id, tag)` pero nunca se escribieron
  para el tenant B
- CUANDO se lee el offset para el tenant B en el mismo `(projection_id, tag)`
- ENTONCES devuelve ausente — nunca el offset del tenant A

#### Requisito: Las Marcas de Dedup Repetidas Convergen a un Solo Registro

Marcar el mismo `(projection_id, tag, event_id)` como visto más de una vez — secuencial o
concurrentemente — DEBE tener éxito en cada llamada, DEBE dejar exactamente un registro de
dedup para esa identidad, y NO DEBE levantar un error en la repetición.

##### Escenario: Una marca duplicada converge sin error

- DADO que `(projection_id, tag, event_id)` ya fue marcado como visto
- CUANDO se marca como visto nuevamente
- ENTONCES la segunda llamada también tiene éxito, existe exactamente un registro para esa
  identidad, y una verificación de "visto" subsiguiente devuelve verdadero

#### Requisito: La Identidad de Dedup Es Independiente del Tenant

El mismo `event_id` marcado bajo dos tenants distintos para el mismo `(projection_id, tag)`
DEBE tratarse como una sola identidad — el tenant NO forma parte de la identidad de dedup.

##### Escenario: El event_id idéntico de un segundo tenant ya se reporta como visto

- DADO que `event_id` fue marcado como visto para `(projection_id, tag)` bajo el tenant A
- CUANDO el mismo `event_id` se marca como visto bajo el tenant B para el mismo
  `(projection_id, tag)`
- ENTONCES se reporta como ya visto — la identidad de dedup no varía por tenant

#### Requisito: Las Escrituras de Offset Son "Última Escritura Gana"

Una escritura sobre el offset de una proyección DEBE sobrescribir el valor previamente
almacenado, sin compare-and-swap, sin verificación de offset-previo-esperado, y sin detección
de una sobrescritura concurrente. Esto es una implementación fiel del propio contrato de
escritura del offset store, no una limitación del adaptador.

##### Escenario: Una escritura posterior sobrescribe silenciosamente a una anterior

- DADO un offset ya escrito para `(projection_id, tag, tenant)`
- CUANDO se emite una segunda escritura para la misma identidad con un valor distinto, sin
  ninguna coordinación de orden entre ambos escritores
- ENTONCES el valor almacenado pasa a ser el de la última escritura, sin error y sin ninguna
  señal de conflicto para ninguno de los dos escritores

#### Requisito: Ambos Stores de Progreso Se Reportan a Sí Mismos Como Durables

El offset store y el dedup store DEBEN reportarse a sí mismos como durables ambos, y una
composición que declara el perfil de durabilidad de producción y registra este par a través
del punto de registro de progreso del read-side existente DEBE construirse exitosamente sin
ningún cambio en la lógica de validación propia de ese perfil.

##### Escenario: Una composición de perfil de producción pasa por durabilidad real

- DADA una composición que declara el perfil de durabilidad de producción
- CUANDO registra este par durable a través del punto de registro de progreso del read-side
  existente
- ENTONCES la composición tiene éxito porque ambos stores se reportan a sí mismos como
  durables, no por un sustituto exclusivo de tests

#### Requisito: El Tenant Es Parte Obligatoria de la Identidad del Offset

Cada registro de offset persistido DEBE portar un valor de tenant concreto. El almacenamiento
persistido de offsets NO DEBE aceptar, y NUNCA DEBE contener, un tenant ausente/nulo en un
registro de offset — a diferencia del manejo de tenant anulable/systemwide usado en otras
partes de este framework para los stores del lado de escritura, que no aplica aquí.

##### Escenario: Un registro de offset siempre porta un tenant concreto

- DADO que una proyección escribe un offset
- CUANDO la escritura se persiste
- ENTONCES el campo de tenant del registro almacenado contiene el valor de tenant concreto
  para el que se hizo la escritura — nunca un valor ausente/nulo

#### Requisito: El Crecimiento del Almacenamiento de Dedup Es Ilimitado en Esta Capability

Esta capability NO DEBE incluir ningún mecanismo de purga, tiempo de vida (TTL), ni
eliminación para los registros de dedup. El almacenamiento de dedup crece monótonamente con
la cantidad de eventos únicos procesados por una proyección, sin un límite superior, mientras
solo esta capability lo gobierne. Esta es una limitación explícita y nombrada, no una omisión.

##### Escenario: Los registros de dedup se acumulan sin eliminación automática

- DADO que una proyección procesó muchos eventos únicos a lo largo del tiempo
- CUANDO se inspeccionan los registros de dedup de esos eventos
- ENTONCES todos siguen presentes — nada en esta capability purgó, expiró, ni eliminó ninguno
  de ellos

#### Requisito: El Camino de Producción de la Aplicación de Referencia Usa el Par Durable

El camino de composición de producción de la aplicación de referencia DEBE registrar un par de
progreso durable y real para su proyección de read-side, en lugar de omitir el progreso del
read-side o sustituirlo por un placeholder no durable.

##### Escenario: La composición de producción ya no omite el progreso del read-side

- DADA la aplicación de referencia componiéndose a sí misma bajo el perfil de durabilidad de
  producción
- CUANDO se alcanza su punto de registro de progreso del read-side
- ENTONCES registra un par durable real — no un valor ausente, y no un placeholder no durable

#### Requisito: La Restricción de Adopción de Escritor Único Está Documentada a Nivel de Adaptador

La documentación pública de los adaptadores de esta capability DEBE indicar, en palabras que
un operador pueda leer, que la operación segura depende de escritor-único-por-`(projection_id,
tag, tenant)`, y NO DEBE presentar una configuración de proyección multi-réplica como
oficialmente soportada.

##### Escenario: La documentación del adaptador enuncia la restricción de adopción

- DADO un operador leyendo la documentación pública a nivel de adaptador de esta capability
- CUANDO busca orientación sobre ejecutar réplicas concurrentes de la misma proyección
- ENTONCES la documentación enuncia explícitamente la restricción de adopción de
  escritor-único-por-`(projection_id, tag, tenant)`, y no describe una configuración
  multi-réplica como soportada

### No-Objetivos

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
