
# terminology scratch
skein/field - scope
knot - node
rope - connection
head - the input end of a knot
tail - the output of a knot
loose head - the head of a knot with nothing connecting to it
loose tail - the tail of a knot with nothing connected to it
tucks, thimble, eyelet? - connectors
unravel - pullback function for reverse inference
bind - captures
bottom - Möbius? lol


load, strand, bead, weave, filament? - value? maybe just use value, or how about bead? maybe strand could be used for promitives and bulit ins?

other fun rope terms you can use:
kink
slack (lazy eval lol)
tension (push forward eval)
braid
fray (loose end)
cleat (golabl outputs)
tether
flemish
snag
tangle (already used)
unreeve
grommet
sheave
pulley


?? - multiplexed otuput nod

# TODO scratch

- node rendering styles, so detail out how we visually distinguish different types of function nodes

- operators and operator precedence, we need this info to do the reverse mapping, I guess you can just enforce one style with a linter which we sorta need to do to keep reverse mapping manageable.




# Core Primitives

## valueboxes

a databox represents valuetypes, they show up in a few ways

- as a literal node
- as a node input: must show variable name
- as a node input with collapsed literal node 
- as a live preview when clicking on a connection

### valuebox content types

there are many value types. The interface must show the type, and in the literal cases, conveniently display the values and allow them to be edited.

- primitive
- record
- ADT
- built-in containers: containers are ADTs but they should have their own custom intefrace
- function types
- (future) custom types: containers are ADT


#### generics
types may take on generic and interface types (may be come more inferred or concrete as stuff gets connected to it)


## node

### literal nodes

literal nodes contain some special UI input field (num filed string field, dropdown, etc) that allows you to set the literal value. They have one output connector.

literal nodes can be "nubbed" into a connector so that a complete node + connection is not shown. All literal nodes are nubbed by default unless they have been let bound to a variable. You can add an annotation to make literal nodes not be nubbed `@no_nub`
`

visually, a nub portrudes out from the input connector outside fo the node by a half semicircel, and the literal input field is in a pillbox shape with the portruding half semicricle as one of its ends.

some datatypes may have special literal syntax (e.g. maps, lists, maybe user defined ones) 
if the input field is one lineable, then it can be nubbed, otherwise nubbing is not supported.

maps, and lists of literals by default come with their special literal input fields.

### let bindings (value nodes)

a let binding binds a value type to a node, it is directly connected to a connection (not via an input connector on the node, the node itself is connected to the connection)

let binding nodes contain the name of the variable that the value is bound to

let bindings have a singel "named" output node, these output nodes are specila as they can be multiplexed (multiple conectors coming out of it)

value nodes may also be symbolicated (pick better term) in the scope, allowing it to be created from the aether

```
--->🐴 (symbolicated let binding)
...
(created from the aether) 🐴--->someinputnode
```

TODO consider having a special let bound literal node, if not, it's just a regular let bound node with a nubbed literal input which is fine too I guess.


### function nodes

functions can be thought of in 2 cases

1. completely called to produce an output value 
2. partially called to produce a boxable incomplete callsite node

nodes must carefully distinguish between these 3 cases

1 and 2 are referred to as usage nodes (also referred to as callsite node for functions)

node that 2 may be boxed to produce a function definition node

#### pillboxed function nodes  (slubs)

functions nodes fo type `x -> y` can be "slubbed" meaning they get a more inline representation on as a "pill" looking thing dircetly on the connector rather than a full node with input and output. Slubs can be chained together to form... caterpillars? many built in functions are set to be slubbed by default, all user defined functions are not slubbed by default and can be annotated to be slubbed by default `@slub_by_default`. callsite nodes can be made to be slubbed or not slubbed (overriding the default) with an annotation `@slub`.

#### simple function nodes (twists?)

functions can be flagged sa simple which makes them render a little more compatly... NAH but at least color some built in function sdifferently maybe

#### ADT ctor nodes

algebraic data type ctor nodes  have a drop down to toggle which exclusive variant is being used to construct it. Otherwise, they are just function nodes. 

while the editor is open (i.e. not serialized) it attepmts to "Remmeber" what was connected to teh previous variants and restores it if you toggle back to it


### boxed nodes (knots)

nodes can be boxed to produce a scope, depending on the contents of the box, the boxing node may be of different types

- a function definition node if it has open node boundary input/output connections
- a value node if it has no node boundary input connections
- a mondaic context node if it produces a monadic output value type (node must be explicitly tagged as supporting monadic context only if this condition is met and this enables monadic bind connectors)

TODO if funciton nodes are knots, what are primitive nodes?

#### function definition nodes 

function definition nodes represent lambdas in code, or named functions if let bound to a varibale

- definition nodes are actually literal nodes containing a function value type
- definition nodes can be created by boxing partially called callsite nodes
- definition nodes have a specila interface allowing function value nodes to be pulled out of it


#### monadic context nodes

:O mondaic context nodes enable binding connectors allowing monadic outputs to be conecting using bind connectors to to non monadic inputs so lang as the final output of the context node is in the monadic context

connections with 

### connectors

a callsite node may have many typed input connectors and one typed output connector

a callsite node may have generic connectors and become more typed as inputs and outputs get added to it

generic connectors can chain together in keeping their generic-ness. They only become concrete when connected to inputs/outputs that force its type to be conceret.

TODO consider entire program as a node, then maybe we allow multiple output connectors)


#### expanding connectors (enable daisy chaining)

TODO some built in nodes need to support epanding connectors e.g. maps and lists so that you don't need to chain a bunch of append/insert nodes together to build a list from a static set of inputs.

Once you fill in one connector, another optionla connector opens up. the node is still treated as fully connected in this case.

easy enough for built ins, we should allow user defined types to support this interface too someday. I guess in general, anything of the type

`fun :: a -> b -> b`

can be annotated as `@enable_daisy_chaining` " or whatever which enables this special interface which really just 

`... fun a1 . fun a2 . fun a3 $ b` or jsut `b` if there is no input

### combined nodes

combined nodes are a special treatement of definition nodes and callsite nodes combined into one. expressions representing nodes that can be combined can be annotated with `@combine` to enable this behavior. THere are a few cases:



1. If a definition node has only one callsite, and the callsite is completely connected (all inputs connected)

normal case, this is basically making a lambda and appyling it to something. 

disconnected output is fine, it's just an unused output, could be rpresented in code as `let _ = \x -> ....`


2. if a definition node has only one calliste, and it is only partially connected inputs

this is an intermediary state, it can be boxed again and converted into a function definition node from a curried function. 

we could consider treating this like a loose end of a non trivial let binding
i.e. `let _ = (\x y z -> ....) $ input1 _ input2` <- not real syntax, would be for intermediate state only if we chose to serialize invalid loose end states.

3. a partially connected combined node that got converted (boxed) into a curried funnctino definiton node

or maybe don't do this, it may be better to NOT do combined nodes in this case. Just force the explicit split definition node -> callsite node and then boxing the one partially connecetd callsite node into a curried function definition node. The cons of this approach is that you will need to do this split automatically for the user when they convert the partially conected combined node into a boxed function definition node and there may need to be special UI case for this.


## connection

a connection is a line drawn from one "output" to one "input"

### loose connections

connections can just be dangling around as a inctermediary non-serialized state. In these cases, the loose ends cna be converted into a let binding or into a callsite node in the case of function types.

in code we can choose to 
- represent loose ends as unnamed let bindings i.e. `_ = ...`
- treat them as an invalid intermediary unserialized state that only exists on the node graph side of things

### node boundary interfacing connections (currying)
funcution definition nodes may have inputs and outputs within the node that connect to the nodes within the outer box node. 

### node boundary crossing input connections (captures)
function definition nodes may have connections permeate them
this forces the function definition node to be in the deepest scope of all captures, although in pracitce, the scope is determined by whatever scope one is in when the node got created and capturing from inaccesible scopes is forbidden by the UI.

QUESTION: do we allow node boundary crossing output connections? this is useful for boxing nodes just for the aske of organization



## recursion

in order to do recusion you must let bind the function into a variable, then using the let bound function inside the function looks like a capture

what does something like
`fix f = let x = f y, y = x in x` look like in the node graph?


## the global scope

TODO how to access built ins from the global scopes? in particular, we need to be able to create callsite nodes from bulit in functions, and we also need to be able to use bulit in functions are function value nodes to use as inputs into higher order functions


### imports

there is some menu somewhere that lets you add / remove imported modules. imported modules are always sorted by name. tangle doesn't care about import qualifications, so all qualifications are treated the same in the viewer. In the reverse mapping, things are not qualified if they don't need to be, otherwise qualified, or we could do always fully qualified which might be easier to miplement.



## builtin node unique behavior

### literal nodes

have a drop down to choose its type

### operator nodes

have a drop down to chooes its type

### list literal nodes

we support list and map literal syntax, these get represented as daisy chained ctors with nubbed inputs. If a map/list ctor node with all inputs are nubbed, then it is represteed in code as a literal, otherwise it gets represented using the usual unsugard repeated `:` application

### tuple nodes

tuple nodes are function nodes and have a configuration dropdown that lets you choose # of elements effectively changing the tuple function node to a different ctor/output type

# Examples

