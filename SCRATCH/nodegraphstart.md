
skein/field - scope
knot - node
rope - connection
head - the input end of a knot
tail - the output of a knot
loose head - the head of a knot with nothing connecting to it
loose tail - the tail of a knot with nothing connected to it
unravel - pullback function for reverse inference

load or strand? - value? maybe just use value, or how about bead?
?? - multiplexed otuput nod
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

### boxing nodes

nodes can be boxed to produce a scope, depending on the contents of the box, the boxing node may be of different types

- a function definition node if it has open node boundary input/output connections
- a value node if it has no node boundary input connections
- a mondaic context node if it produces a monadic output value type (node must be explicitly tagged as supporting monadic context only if this condition is met and this enables monadic bind connectors)


### function nodes

functions can be thought of in 3 cases

1. completely called to produce a value
2. partially called to produce a boxable incomplete callsite node
3. as a value node

nodes must carefully distinguish between these 3 cases

1 and 2 are referred to as usage nodes

2 may be boxed to produce a definition node

- usage node (also referred to as callsite node for functions)
- definition nodes can be created by boxing partially called callsite nodes
- definition nodes have a specila interface allowing function value nodes to be pulled out of it

definition nodes may also be pulled from the scope by the name its bound to, for global scope, this is the only way to obtain callsite nodes, for local scopes, both methods are possible 

### monadic context nodes

:O mondaic context nodes enable binding connectors allowing monadic outputs to be conecting using bind connectors to to non monadic inputs so lang as the final output of the context node is in the monadic context

connections with 

### connectors

a callsite node may have many typed input connectors and one typed output connector

a callsite node may have generic connectors and become more typed as inputs and outputs get added to it

generic connectors can chain together too keeping their generic-ness

TODO consider entire program as a node, then maybe we allow multiple output connectors)

## connection

a connection is a line drawn from one "output" to one "input"

### loose connections, connections can just be dangling around as a inctermediary non-serialized state. In these cases, the loose ned cna be converted into a let binding or into a callsite node in the case of function types.

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

# Examples

