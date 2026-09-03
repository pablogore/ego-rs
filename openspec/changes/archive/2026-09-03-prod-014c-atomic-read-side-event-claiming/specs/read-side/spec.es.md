# Delta para read-side

> Documento acompañante para revisión. La fuente de verdad canónica es
> `spec.md` (identificadores 1:1). Spec base de la capacidad:
> `openspec/specs/read-side/spec.md` (sección `Capability: read-side`), los
> dos requisitos de PROD-014B agregados ahí. Este delta toca solo esos dos;
> el tercer requisito de PROD-014B, "Durable Dedup Bookkeeping Does Not
> Imply Exactly-Once Handler Execution," no se ve afectado y DEBE
> sobrevivir sin cambios — no se reproduce aquí porque nada sobre él
> cambia.

Alcance: PROD-014C. La restricción de adopción de escritor único que
PROD-014B documentó como externa y no forzada ahora es forzada por la
nueva capacidad `read-side-event-claiming`. Ambos títulos de requisito se
renombran porque sus títulos previos afirmaban la ausencia de aplicación
forzada — mantener el título viejo con un cuerpo de texto nuevo y
contradictorio tergiversaría la capacidad; `sdd-archive` DEBE reemplazar
el bloque del título viejo con el renombrado, no dejar ambos presentes.

## Requisitos RENOMBRADOS

### Requisito: Prevention of Double Handler Execution Rests on an Explicit, Unenforced Single-Writer Adoption Constraint → Prevention of Double Handler Execution Is Enforced By Atomic Claiming Across Replicas

(Razón: la restricción pasó de ser una convención de adopción externa y no
forzada a ser un mecanismo que este framework mismo aplica —
el claim atómico de `read-side-event-claiming`. El título viejo afirmaba
"unenforced", lo cual ya no es cierto.)
(Migración: cualquier documento o comentario de código que cite el título
viejo por nombre DEBE actualizarse al nuevo título; la garantía que nombra
cambió de ausente a forzada, no simplemente se reformuló.)

### Requisito: The Concurrency Gap Has a Named, Distinct Follow-Up → The Concurrency Gap Named In PROD-014B Is Discharged By Atomic Claiming

(Razón: el seguimiento al que apuntaba el título viejo — PROD-014C — ya se
entregó. El vacío está cerrado, no simplemente rastreado bajo un
seguimiento nombrado.)
(Migración: cualquier documento o comentario de código que cite
"PROD-014C" como un seguimiento abierto DEBE actualizarse para indicar que
el vacío está saldado, según `read-side-event-claiming`.)

## Requisitos MODIFICADOS

### Requisito: Prevention of Double Handler Execution Is Enforced By Atomic Claiming Across Replicas

La prevención de la ejecución doble del handler para el mismo evento, entre
réplicas concurrentes de la misma proyección, DEBE ser aplicada por el
mecanismo de claim atómico de la capacidad `read-side-event-claiming` —
nunca dejada como una restricción de adopción externa y no forzada. Como
máximo un worker DEBE sostener un claim de procesamiento válido para un
`(projection_id, tag, tenant)` dado a la vez; un worker sin un claim válido
NO DEBE invocar el handler para ese stream. Esta aplicación limita solo el
conteo de ejecuciones del handler — no limita lo que hace el propio efecto
externo del handler (ver "Durable Dedup Bookkeeping Does Not Imply
Exactly-Once Handler Execution", sin cambios, y los No-Objetivos de
`read-side-event-claiming`).
(Previamente: afirmaba que esto dependía de una restricción de adopción de
escritor único externa y no forzada por `(projection_id, tag, tenant)`, sin
ningún mecanismo de elección de líder, lock, lease o fencing que la
aplicara.)

#### Escenario: Un despliegue de dos réplicas está dentro de la garantía, y forzado

- DADO dos réplicas del mismo proceso de proyección corriendo
  concurrentemente contra el mismo `(projection_id, tag, tenant)`
- CUANDO ambas intentan procesar al mismo tiempo
- ENTONCES como máximo una sostiene un claim válido e invoca el handler; la
  otra es rechazada y nunca invoca el handler en ese ciclo — esta
  configuración está dentro de la garantía de esta capacidad, no fuera de
  ella

#### Escenario: La aplicación forzada nunca afirma manejo exactamente-una-vez

- DADO un worker que sostiene un claim válido para un batch completo
- CUANDO crashea después de que el handler tuvo éxito pero antes de que el
  batch quede completamente registrado, y luego se reanuda
- ENTONCES el handler PUEDE ejecutarse de nuevo para esos eventos; la
  aplicación forzada de exclusión entre réplicas no convierte la ejecución
  al-menos-una-vez del handler en exactamente-una-vez, y ninguna
  documentación puede describirlo así

### Requisito: The Concurrency Gap Named In PROD-014B Is Discharged By Atomic Claiming

El vacío que PROD-014B nombró entre el registro durable de dedup y la
prevención de la ejecución doble del handler entre réplicas DEBE tratarse
como saldado: `read-side-event-claiming` aplica la exclusión antes de que
el handler se ejecute. La documentación que describa este vacío como
abierto, sin dueño, o pendiente de un seguimiento DEBE tratarse como
obsoleta y corregirse.
(Previamente: afirmaba que el vacío DEBÍA registrarse como un seguimiento
distinto y nombrado — PROD-014C — Atomic Read-Side Event Claiming — en
lugar de plegarse dentro del alcance de esta capacidad o dejarse sin dueño
en silencio.)

#### Escenario: Un lector encuentra el mecanismo saldado, no un seguimiento pendiente

- DADO un lector de la documentación de esta capacidad buscando cómo se
  previene la ejecución doble del handler entre réplicas
- CUANDO busca el mecanismo responsable
- ENTONCES encuentra el claiming atómico, aplicado por
  `read-side-event-claiming` — no un seguimiento nombrado pero no
  entregado
