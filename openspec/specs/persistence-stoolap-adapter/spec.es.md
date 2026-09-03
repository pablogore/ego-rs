# Spec: `persistence-stoolap-adapter` (Capability Nueva)

> Documento acompañante para revisión. La fuente de verdad canónica es `spec.md` (identificadores
> de requisito y escenarios 1:1). Fuente del contenido de los requisitos: `proposal.md` de
> STOOLAP-S1 (D-1..D-12, IS-1..IS-6, R1..R14) y `design.md` (EC-1..EC-7, AD-1..AD-11, OQ-1..OQ-3).
> Esta capability describe el contrato **observable** que ve quien llama a `Repository<A>` cuando
> se usa la implementación respaldada por Stoolap — qué devuelve un save, load o delete, no cómo
> se calcula. Ningún requisito aquí nombra un mecanismo de almacenamiento, un lenguaje de
> consulta, un índice o una codificación interna; esas decisiones pertenecen a `design.md` y
> pueden cambiar sin que este spec cambie, siempre que cada escenario siguiente se siga cumpliendo.

## Propósito

El contrato observable de que existe una implementación de `ego_persistence_api::persistence::Repository<A>`
respaldada por Stoolap; que delimita los agregados por tenant, con el alcance systemwide (sin
tenant) aislado de cada tenant concreto e igual a sí mismo en todas las llamadas; que aplica
concurrencia optimista, reportando toda carrera perdida como el mismo resultado de conflicto; que
un guardado confirmado sobrevive a un reinicio no controlado del proceso que lo realizó; que sus
resultados de error nunca exceden los cuatro ya provistos por `Repository<A>`; y que su
comportamiento observable externamente es indistinguible del de las implementaciones en memoria y
respaldada por PostgreSQL para cada escenario que cubre el arnés de conformidad compartido.
También establece, como un límite explícito y no como una omisión silenciosa, que las garantías
de esta capability se sostienen únicamente dentro de un único proceso propietario y no hacen
ninguna promesa sobre dos o más procesos de sistema operativo separados que compartan el mismo
almacén.

Esta capability no cubre ningún otro almacén respaldado por Stoolap (`EventStore`, `Snapshot`,
`OperationReservationStore`, `OffsetStore`, `DedupStore`), ninguna abstracción genérica de backend
de almacenamiento, ningún backend más allá de Memory/PostgreSQL/Stoolap, ni ningún cambio al
conjunto de métodos, límites o tipo de error propios de `Repository<A>`.

## Requisitos

### Requisito: R1 — Existe Una Implementación de Repository Respaldada Por Stoolap Y Hace Round-Trip De Un Agregado

DEBE existir una implementación de `Repository<A>` respaldada por Stoolap, y quien la llame DEBE
poder guardar un agregado nuevo, recuperarlo con contenido idéntico, guardar una actualización con
la versión avanzando exactamente en uno en cada guardado exitoso, y posteriormente eliminarlo.

#### Escenario: Un guardado de agregado nuevo comienza en la versión uno

- DADO un agregado que nunca ha sido guardado
- CUANDO quien llama lo guarda con una versión esperada de cero
- ENTONCES el guardado tiene éxito y reporta la versión uno

#### Escenario: Los guardados exitosos secuenciales avanzan la versión exactamente en uno cada vez

- DADO un agregado ya guardado en la versión uno
- CUANDO quien llama lo guarda de nuevo con una versión esperada de uno
- ENTONCES el guardado tiene éxito y reporta la versión dos

#### Escenario: Un agregado recuperado coincide con el contenido guardado más recientemente

- DADO un agregado guardado, y luego guardado de nuevo con contenido actualizado
- CUANDO quien llama lo recupera
- ENTONCES el contenido recuperado coincide con el guardado más reciente, no con uno anterior

### Requisito: R2 — Memory, PostgreSQL Y Stoolap Satisfacen Un Único Contrato Conductual Compartido

Las implementaciones de `Repository<A>` en memoria, respaldada por PostgreSQL y respaldada por
Stoolap DEBEN ser evaluadas contra un único conjunto idéntico de escenarios de conformidad
compartido, no contra tres lecturas separadas del contrato. Aprobar ese arnés compartido DEBE ser
requerido de las tres implementaciones para cada escenario que define, sin variante por
implementación y sin escenario omitido. (El R6 siguiente nombra el único escenario excluido
deliberadamente de este arnés, y explica por qué.)

#### Escenario: Los mismos escenarios de conformidad se ejecutan contra las tres implementaciones

- DADO un arnés de conformidad compartido que cubre el versionado de guardado nuevo, el avance de
  versión secuencial, el conflicto por versión desactualizada, el round-trip de recuperación, el
  no-encontrado en recuperación y eliminación ausentes, la eliminación real, el rechazo de tenant
  faltante y los tres escenarios de aislamiento de tenant del R3
- CUANDO ese arnés se ejecuta contra las implementaciones en memoria, respaldada por PostgreSQL y
  respaldada por Stoolap
- ENTONCES cada escenario se aprueba de forma idéntica en las tres, sin ningún escenario omitido
  ni variado por implementación

### Requisito: R3 — Los Alcances De Tenant Están Aislados Entre Sí, Incluyendo El Alcance Systemwide

Un agregado guardado bajo un alcance de tenant NUNCA DEBE ser visible bajo, confundirse con, o
ser sobrescrito por un guardado bajo un alcance de tenant diferente o bajo el alcance systemwide
(sin tenant), incluso cuando ambos comparten la misma identidad de agregado. Esto DEBE incluir al
propio alcance systemwide, que hace round-trip a través de guardar, recuperar, guardar de nuevo y
eliminar exactamente como lo hace un alcance de tenant nombrado.

#### Escenario: El alcance systemwide hace round-trip a través de guardar, recuperar, guardar y eliminar

- DADA una identidad de agregado guardada sin tenant especificado (alcance systemwide)
- CUANDO se guarda, se recupera, se guarda de nuevo con una versión avanzada, y luego se elimina
- ENTONCES cada paso tiene éxito exactamente como lo haría para un alcance de tenant nombrado, y
  el agregado desaparece después de la eliminación

#### Escenario: Dos tenants diferentes que comparten una identidad de agregado no colisionan

- DADA la misma identidad de agregado guardada de forma independiente bajo dos alcances de tenant
  diferentes
- CUANDO cada uno se recupera
- ENTONCES cada uno devuelve únicamente el contenido guardado bajo su propio tenant, y ninguno es
  visible bajo el alcance systemwide

#### Escenario: Un alcance de tenant y el alcance systemwide que comparten una identidad de agregado no colisionan

- DADA la misma identidad de agregado guardada de forma independiente bajo un alcance de tenant
  nombrado y bajo el alcance systemwide
- CUANDO se elimina el agregado con alcance systemwide
- ENTONCES el agregado con alcance de tenant permanece intacto y sin afectar

### Requisito: R4 — Un Identificador De Tenant Vacío Es Rechazado, Nunca Coaccionado

Quien llame con una cadena vacía como identificador de tenant DEBE recibir un rechazo
`PersistenceError::MissingTenant` tanto en guardar como en recuperar y eliminar; NUNCA DEBE ser
tratado silenciosamente como el alcance systemwide ni como un tenant válido.

#### Escenario: Un identificador de tenant vacío es rechazado en cada operación

- DADA una cadena vacía pasada como identificador de tenant
- CUANDO quien llama intenta guardar, recuperar o eliminar un agregado bajo ella
- ENTONCES cada operación es rechazada como `PersistenceError::MissingTenant`, y ninguna tiene
  éxito

### Requisito: R5 — Una Versión Esperada Desactualizada Es Rechazada Como Conflicto, De Forma Veraz

Cuando quien llama guarda un agregado existente con una versión esperada que ya no coincide con
la versión actualmente almacenada del agregado, el guardado DEBE ser rechazado como
`PersistenceError::Conflict`, reportando tanto la versión que quien llama esperaba como la versión
realmente almacenada, en lugar de ser aceptado silenciosamente o reportado de forma incorrecta.

#### Escenario: Guardar con una versión esperada desactualizada es rechazado

- DADO un agregado actualmente almacenado en la versión dos
- CUANDO quien llama lo guarda con una versión esperada de uno
- ENTONCES el guardado es rechazado como un conflicto que reporta esperado uno y actual dos, y el
  agregado almacenado permanece sin cambios

### Requisito: R6 — Un Agregado Nuevo Rechaza Una Versión Esperada Distinta De Cero, Coincidiendo Con La Semántica Documentada

Cuando quien llama guarda un agregado que nunca ha sido guardado antes (no existe un guardado
previo para ese alcance e identidad) usando una versión esperada distinta de cero, la
implementación respaldada por Stoolap DEBE reportar un conflicto de versión en lugar de aceptar
la escritura — coincidiendo tanto con el comportamiento de la implementación en memoria como con
el propio contrato documentado de `Repository<A>` ("usar 0 para agregados nuevos"). Este escenario
NO DEBE aparecer en el arnés de conformidad compartido del R2: las dos implementaciones
previamente publicadas ya se sabe que discrepan exactamente en este caso (la implementación
respaldada por PostgreSQL actualmente acepta la escritura en lugar de reportar un conflicto), y
reconciliar esa discrepancia está fuera del alcance de esta capability, rastreado por separado
como su propio seguimiento. Excluir el escenario del arnés compartido no es una afirmación de que
el comportamiento de la implementación respaldada por Stoolap sea incorrecto; es una afirmación de
que el arnés compartido solo garantiza aquello en lo que las tres implementaciones están
realmente obligadas a coincidir hoy.

#### Escenario: Un agregado completamente nuevo guardado con una versión esperada distinta de cero es rechazado como conflicto

- DADO que nunca se ha guardado un agregado bajo un alcance de tenant e identidad de agregado
  determinados
- CUANDO quien llama guarda ese agregado con una versión esperada distinta de cero
- ENTONCES el guardado es rechazado como un conflicto que reporta una versión actual de cero, no
  aceptado

#### Escenario: Este caso no se ejerce en el arnés de conformidad compartido entre backends

- DADO el arnés de conformidad compartido definido en el R2
- CUANDO se inspecciona la lista de escenarios del arnés
- ENTONCES no contiene ningún escenario que cubra un agregado nuevo guardado con una versión
  esperada distinta de cero, porque ya se sabe que las implementaciones previamente publicadas
  discrepan en este caso exacto, independientemente de esta capability

### Requisito: R7 — Recuperar O Eliminar Un Agregado Ausente Reporta No Encontrado

Recuperar o eliminar un agregado que nunca fue guardado, o que ya fue eliminado, DEBE ser
reportado como `PersistenceError::NotFound`, en lugar de un resultado vacío, un valor por defecto,
o un error que represente incorrectamente la causa.

#### Escenario: Recuperar un agregado que nunca fue guardado reporta no encontrado

- DADA una identidad de agregado que nunca ha sido guardada
- CUANDO quien llama la recupera
- ENTONCES la recuperación es rechazada como no encontrado

#### Escenario: Eliminar un agregado que nunca fue guardado reporta no encontrado

- DADA una identidad de agregado que nunca ha sido guardada
- CUANDO quien llama la elimina
- ENTONCES la eliminación es rechazada como no encontrado

### Requisito: R8 — Eliminar Remueve El Agregado De Forma Permanente

Eliminar un agregado DEBE dejarlo genuinamente ausente: una recuperación posterior DEBE reportar
no encontrado, no un valor vacío o marcado como eliminado (tombstone).

#### Escenario: Un agregado eliminado realmente desaparece

- DADO un agregado que ha sido guardado
- CUANDO quien llama lo elimina y luego intenta recuperarlo
- ENTONCES la recuperación es rechazada como no encontrado

### Requisito: R9 — Un Guardado Confirmado Sobrevive A Un Reinicio No Controlado Del Proceso

Una vez que un guardado se ha completado con éxito, sus datos DEBEN estar presentes después de que
el proceso que lo realizó deje de ejecutarse (incluyendo una detención no controlada) y un proceso
nuevo reabra el mismo almacén en la misma ubicación.

#### Escenario: Los datos de un guardado completado están presentes después de que el proceso se reinicia

- DADO un guardado que se ha completado con éxito contra un almacén en una ubicación determinada
- CUANDO el proceso se detiene y un proceso nuevo reabre el almacén en esa misma ubicación
- ENTONCES recuperar el agregado guardado devuelve el contenido que fue guardado, sin cambios

### Requisito: R10 — Todo Conflicto De Escritura Se Reporta A Través Del Mismo Resultado De Conflicto Existente

Ya sea que un guardado sea rechazado por una versión esperada desactualizada o porque perdió una
carrera contra una escritura concurrente al mismo agregado, ambos casos DEBEN ser reportados a
través del mismo resultado idéntico `PersistenceError::Conflict` que quien llama ya sabe manejar
recuperando de nuevo y reintentando. NO DEBE introducirse ningún resultado de error nuevo,
adicional o de forma diferente para ninguno de los dos casos, y quien llama NO DEBE poder
distinguir "versión desactualizada" de "perdió una carrera" únicamente por la forma del resultado.

#### Escenario: Una carrera de escritura concurrente se reporta de la misma forma que una versión desactualizada

- DADOS dos llamadores intentando guardar concurrentemente el mismo agregado existente
- CUANDO ambos intentos se realizan casi al mismo tiempo
- ENTONCES exactamente un guardado tiene éxito, y el otro es rechazado con el mismo resultado de
  conflicto que el R5 describe para una versión esperada desactualizada

### Requisito: R11 — Ningún Detalle Interno De Almacenamiento Es Visible Jamás Para Quien Llama

Nada sobre cómo se representa internamente un alcance de tenant o el alcance systemwide DEBE ser
expuesto jamás a quien llama — ni en un valor devuelto, ni en un mensaje de error, ni en ninguna
diferencia de comportamiento que quien llama pudiera observar. La visión que tiene quien llama del
alcance de tenant DEBE ser idéntica en las tres implementaciones de `Repository<A>` (R2).

#### Escenario: Ningún error o valor devuelto revela la representación interna del alcance

- DADA cualquier llamada de guardar, recuperar o eliminar en cualquier alcance de tenant,
  incluyendo el alcance systemwide
- CUANDO se inspecciona el resultado de la llamada o cualquier error que produzca
- ENTONCES nada en él revela cómo se representó internamente el alcance, y el comportamiento
  visible para quien llama es indistinguible del de las otras dos implementaciones de
  `Repository<A>`

### Requisito: R12 — El Acceso Concurrente Está Garantizado Únicamente Dentro De Un Único Proceso Propietario

Las garantías de la implementación respaldada por Stoolap — incluyendo el aislamiento de tenant
(R3) y la detección de conflictos por concurrencia optimista (R5, R10) — DEBEN sostenerse
únicamente entre llamadores dentro de un único proceso propietario. Esta capability **no ofrece
ninguna garantía**, y NO DEBE asumirse, para un comportamiento correcto o seguro cuando dos o más
procesos de sistema operativo separados acceden concurrentemente al mismo almacén subyacente. Un
despliegue que necesite que múltiples procesos compartan los datos de un mismo almacén está fuera
del alcance de esta capability.

#### Escenario: Los llamadores concurrentes dentro de un proceso ven una detección de conflictos correcta

- DADOS dos intentos concurrentes de guardado contra el mismo agregado, realizados por dos
  llamadores dentro de un único proceso propietario
- CUANDO ambos intentos compiten entre sí
- ENTONCES exactamente uno tiene éxito y el otro se reporta como conflicto, según el R10

#### Escenario: El acceso concurrente multi-proceso es una no-garantía explícita

- DADO un despliegue que ejecuta dos o más procesos de sistema operativo separados contra el
  mismo almacén subyacente
- CUANDO esos procesos acceden al almacén de forma concurrente
- ENTONCES esta capability no documenta ninguna garantía de comportamiento correcto o seguro para
  ese escenario, y dicho despliegue queda fuera del alcance de esta capability
