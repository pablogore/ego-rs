# Delta para application-composition

> Documento acompañante para revisión. La fuente de verdad canónica es
> `spec.md` (identificadores 1:1). Spec base de la capacidad:
> `openspec/specs/application-composition/spec.md`. Este delta sigue
> exactamente la forma del requisito existente "Duplicate Effect Store
> Registration Through AppBuilder Fails Closed" de ese spec.

Alcance: PROD-014A. Agrega un punto de registro en la raíz de composición
para el par de progreso durable de una proyección, indexado por
`projection_id` (D-3). La superficie pública exacta (dos métodos, un
método combinado, una struct de registro) es una decisión de
`design.md`; este delta especifica solo el comportamiento observable de
registro, validación y rechazo.

## Requisitos AGREGADOS

### Requisito: Registro del Par de Progreso Durable del Read-Side, Indexado por Projection ID, con el Par como Unidad

`AppBuilder` DEBE proveer un punto de registro en la raíz de composición
para el par de progreso durable de una proyección — su `OffsetStore` y su
`DedupStore` juntos — indexado por `projection_id`. El par DEBE ser la
unidad de registro: un registro que cubra solo uno de los dos stores NO
DEBE ser representable a través de la superficie pública, de modo que una
configuración parcial nunca pueda pasar la validación como si ambos
estuvieran cubiertos. Dos `projection_id` distintos PUEDEN registrar
instancias de store distintas, y también PUEDEN compartir la misma
instancia de store entre proyecciones sin que ese compartir se trate como
un conflicto.

#### Escenario: Dos proyecciones registran pares distintos de forma independiente

- DADO dos `projection_id` distintos
- CUANDO cada uno registra su propio par `OffsetStore`/`DedupStore`
- ENTONCES ambos registros tienen éxito y permanecen distintos en
  `build()`

#### Escenario: El registro parcial de solo un store no es representable

- DADO la superficie pública de registro
- CUANDO una aplicación intenta suministrar solo un `OffsetStore` o solo
  un `DedupStore` para un `projection_id` sin el otro
- ENTONCES la superficie no ofrece forma de hacerlo — el par siempre es
  la unidad suministrada en conjunto

#### Escenario: La misma instancia de store puede compartirse entre projection_ids

- DADO un par de instancias `OffsetStore`/`DedupStore`
- CUANDO se registra para dos `projection_id` distintos
- ENTONCES ambos registros tienen éxito — compartir una instancia entre
  proyecciones no es un conflicto

### Requisito: El Registro Duplicado del Progreso Durable del Read-Side a través de AppBuilder Falla de Forma Cerrada

Registrar un segundo par de progreso durable para el mismo
`projection_id` DEBE fallar de la misma forma en que ya fallan de forma
cerrada `.adapter()`/`.projection()`/`.entity()`/`.effect_store()`:
latcheado como un error de composición y expuesto solo a través del
reporte de errores de composición ya existente de `AppBuilder::build()`,
nunca un sobrescritura silenciosa. Si un error de composición ya está
latcheado por una llamada de registro previa, una llamada de registro de
progreso durable posterior NO DEBE mutar más el estado de registro, y el
error preexistente sigue siendo el que se expone en `build()`.

#### Escenario: El registro duplicado para el mismo projection_id se expone en build, no se reemplaza silenciosamente

- DADO un par de progreso durable registrado dos veces para el mismo
  `projection_id`
- CUANDO se llama a `AppBuilder::build()`
- ENTONCES la construcción falla con un error de composición que
  identifica el `projection_id` duplicado, y el primer par registrado es
  el que habría resuelto si la construcción hubiera tenido éxito

#### Escenario: Un error de composición preexistente no se sobrescribe por una llamada de registro posterior

- DADO un error de composición ya latcheado por una falla de registro
  anterior
- CUANDO se hace una llamada de registro de progreso durable después
- ENTONCES el builder se devuelve sin modificar y el error de composición
  original, no uno nuevo, es el que se expone en `build()`

### Requisito: Un Par de Progreso Durable Registrado Es el Par Que la Proyección Realmente Usa

Registrar el par de progreso durable de una proyección en la raíz de
composición DEBE suministrar las instancias reales de `OffsetStore`/
`DedupStore` que usa la ejecución de esa proyección — no una declaración
paralela a, y potencialmente divergente de, el par que se pasa a
`ProjectionSpec`/`TagSchedulerImpl::spawn`. Una composición NO DEBE poder
registrar un par durable en la raíz de composición mientras un par
distinto y volátil es el que realmente usa la proyección al spawnear.

#### Escenario: El par registrado es el par con el que la proyección spawnea

- DADO una proyección registrada con un par durable en la raíz de
  composición
- CUANDO se compone la ejecución de read-side de esa proyección
- ENTONCES las instancias de `OffsetStore`/`DedupStore` con las que
  spawnea son las mismas instancias registradas en la raíz de composición

#### Escenario: El camino de Production del host de referencia obtiene su par de la raíz de composición

- DADO el camino de composición de Production de `examples/reference-app`
- CUANDO se construyen sus handles de read-side
- ENTONCES el par `OffsetStore`/`DedupStore` se origina en la raíz de
  composición, en lugar de construirse incondicionalmente como
  `InMemoryOffsetStore`/`InMemoryDedupStore` dentro de
  `ReadSideHandles::new()`
