
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
?? - annotation
bundle? - boxing nodes
value - ?
primitive value - ?

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
    - TODO make a list of the different things youw ant styled
- operators and operator precedence, we need this info to do the reverse mapping, I guess you can just enforce one style with a linter which we sorta need to do to keep reverse mapping manageable.
- REORGANIZE
    -application interfaces (program outputs, import selector)
    -graph primitives, nodes, connnectors, 
    -special node types
    -BONUS combined nodes
- consider having entry point functions just be boxed funciton nodes that you fill in (with input and ouputs predefined, can't create new ones) and then module import/export could be done via nodes on the left/right (but that's silly for imports cuz we'd probably rather just pull imported stuff from teh aether.)
- zoom in mode focuses in on any boxed node, the entire application is similarly as such.

# START AGAIN HERE

This spec outlines the Tangle UI and its mapping with the knot programming language

# Core Concepts

There are 3 main concpets in knot

- nodes (knots)
- connectors (head and tails of knots)
- connection (rope)

each may have several variations to represent the entirety of the language.

nodes are quantized typed expressions and are composed with other nodes with connectors and connections to represent expressions.

connectors have types, and these types may be generic gradually become inferred as things get connected to them

# Node

## function node

functions can be thought of in 2 cases

1. completely called to produce an output value 
2. partially called to produce a boxable incomplete callsite node

nodes must carefully distinguish between these 3 cases

1 and 2 are referred to as usage nodes (also referred to as callsite node for functions)

node that 2 may be boxed to produce a function definition node

fuction nodes may ethire be built in/imported or user defined, built in functions nodes are "created from the aether" and user defined ones are boxed from other nodes.

### pillboxed function nodes  (slubs)

functions nodes fo type `x -> y` can be "slubbed" meaning they get a more inline representation on as a "pill" looking thing dircetly on the connector rather than a full node with input and output. Slubs can be chained together to form... caterpillars? many built in functions are set to be slubbed by default, all user defined functions are not slubbed by default and can be annotated to be slubbed by default `@slub_by_default`. callsite nodes can be made to be slubbed or not slubbed (overriding the default) with an annotation `@slub`.

V2 we can support slubbed output connectors inside boxed nodes. This is not really useful except for doing undefined in an unfinished function.

### simple function nodes (twists?)

functions can be flagged sa simple which makes them render a little more compatly... NAH but at least color some built in function sdifferently maybe

### ADT ctor nodes

algebraic data type ctor nodes  have a drop down to toggle which exclusive variant is being used to construct it. Otherwise, they are just function nodes. 

while the editor is open (i.e. not serialized) it attepmts to "Remmeber" what was connected to teh previous variants and restores it if you toggle back to it



### boxed nodes (knots)

nodes can be boxed to produce a scope. The boxed contents must have exactly one loose output end. Depending on the contents of the box, the boxing node may be of different types

- a function definition node if it has open node boundary input/output connections
- a value node if it has no node boundary input connections (a value expression can be annotated `@explicitly_box` to be boxed, monadic value expressions must be tagged `@no_box`)

see connection section as well


#### function definition nodes 

function definition nodes represent lambdas in code, or named functions if let bound to a varibale

- definition nodes are actually literal nodes containing a function value type
- definition nodes can be created by boxing partially called callsite nodes
- definition nodes have a specila interface allowing function value nodes to be pulled out of it


#### monadic context


If the output type of a box is monadic, the boxed node can have monadic context and dose by default (disable with `@no_monadic_context`) 

mondaic context nodes enable binding connectors allowing monadic outputs to be conecting using bind connectors to to non monadic inputs.

connections within a monadic context node can auto bind `m a` into function nodes that have `a` type input using a different connection graphic indicating it is a bind

 (`m a` value) --->>---> (`a -> m b` function node) --->>---> (`m b` value)

nested nodes within a monadic context node do not inherit the monadic context (TODO we could allow capturing monadically bound varibales i.e `a <- return x` but for nwo, just require them geting piped through a `m a` typed input)


#### hidden impl

boxed nodes with no captures can be annotated as `@hidden` which makes everything inside them dissapear in teh UI.
(maybe `@buried` ? or `@obtuse` cuz I liket hat word)


## special syntax nodes

### application entry point node

this is a special boxed function node that must exist for the program to be valid. otheriwes it behaves the same...

### literal nodes

literal nodes contain some special UI input field (num filed string field, dropdown, etc) that allows you to set the literal value. They have one output connector.

literal nodes can be "nubbed" into a connector so that a complete node + connection is not shown. All literal nodes are nubbed by default unless they have been let bound to a variable. You can add an annotation to make literal nodes not be nubbed `@no_nub`
`

visually, a nub portrudes out from the input connector outside fo the node by a half semicircel, and the literal input field is in a pillbox shape with the portruding half semicricle as one of its ends.

some datatypes may have special literal syntax (e.g. maps, lists, maybe user defined ones) 
if the input field is one lineable, then it can be nubbed, otherwise nubbing is not supported.

have a drop down to choose its type


### operator nodes

have a drop down to chooes its type

### list literal nodes

we support list and map literal syntax, these get represented as daisy chained ctors with nubbed inputs. If a map/list ctor node with all inputs are nubbed, then it is represteed in code as a literal, otherwise it gets represented using the usual unsugard repeated `:` application

### tuple nodes

tuple nodes are  just function nodes and have a configuration dropdown that lets you choose # of elements effectively changing the tuple function node to a different ctor/output type

tuple access nodes (get the nth element in a tuple) are specila in that they auto change types based on what gets inserted into it, this is just node sugaring, it code it uses like `\(,,,x)->x` syntax or whatever, lenses and other cute tricks to generically access tuples are omitted.

### record create nodes
record create nodes are just funciton nodes but contain a drop down that lets you choose which record you are creating

### record update nodes
record update nodes are special, they have no ereuired input of the record being updated and then have several opitonal connectors. This maps to the `{ record | field = value, ... }` elm record update syntax. you can still do regular haskell style updateing by creating new record nodes but that sucks in both UI and in code so that's why we have specila record update syntax

### let bound record nodes

let bound record nodes are special! They have mulitple optional output connectors for each of their fields as well as an output connector for the entire record! `record.field` in code is alwasy represented in this way. I guess if you do `.field record` we could have this show up as a regular `.filed` function with `record` as its input? or we could just lint this case away, probbaly better that way.

### if else nodes

looks like a funciton node of type `a -> bool -> a -> a` but maybe different graphic/color cuz its special


### pattern match nodes

these are similar to literal nodes but they have special UI to allow you to match against ADT ctors and take wildcards
VERY SPECIAL TODO figure out what UI is for this...

### case nodes

this noe is special, starts by showing a value, a pattern match, and a output value, once connected, 2 more optional things pop up for another match and output value (if either is connected, the other ones becomes non optional and 2 more optional ones pop up)

actually, after a value is connected, another optional value filed pops up allowing you to do multi case nodes :O

a case node that appears on a function input in a definition node has a UI to tag it as pattern match which converts it into a sugared pattern match syntax on the function. This can only be done for one such case node!

### let bindings (value nodes)

a let binding binds a value type to a node, it is directly connected to a connection (not via an input connector on the node, the node itself is connected to the connection)

let binding nodes contain the name of the variable that the value is bound to

let bindings have a singel "named" output node, these output nodes are specila as they can be multiplexed (multiple conectors coming out of it)

value nodes may also be symbolicated (pick better term) via `@symbolicate(symbol)` in the scope, allowing it to be created from the aether

```
--->🐴 (symbolicated let binding)
...
(created from the aether) 🐴--->someinputnode
```

TODO consider having a special let bound literal node, if not, it's just a regular let bound node with a nubbed literal input which is fine too I guess.

### bulit in, imported, and symoblicated nodes

built in, imported, and symbolicated stuff is basically stuff avaliable in the scope that isn't a primitive but doesn't have an input connector coming into it and isn't a boxed callsite node. So these are built in functions/values and symoblicated let bindings that are "created from the aether" (how abotu pulled from a wormhole)

The built in function callsite nodes can be converted to definitino nodes to be used as higher order function args. They can also be rpresented by boxing the callsite node and in code is tagged as `@explicity_boxed` or something like thta.  These ones are also alowed to be nubbed!

Such stuff is also effectviely hidden by default. You can unhide by adding a `@explicitly_show` or soemthing like that. We might want an alterantiev interface for 


#### application built in nodes

we may have a specila category for application builtints (vs knot bulit ins) as they are likely to get used a not more e.g.

global_settings :: Settings or
read_settings :: IO Settings




# connectors

a callsite node may have many typed input connectors and one typed output connector

a callsite node may have generic connectors and become more typed as inputs and outputs get added to it

generic connectors can chain together in keeping their generic-ness. They only become concrete when connected to inputs/outputs that force its type to be conceret.

TODO consider entire program as a node, then maybe we allow multiple output connectors)

## output connectors

a node must have exactly one output connector and it does not have a name

### disconnected output connector

a disconnected output of `expression` is represented in code as `let _ = expression`

## input connectors
 
a node may have 0 or more input connectors, and these do have a name

## optional input connectors

some connectors are optional (daisy chaining and records, maybe future optional fields?)

## expanding input connectors (enable daisy chaining)

some built in nodes need to support epanding connectors e.g. maps and lists so that you don't need to chain a bunch of append/insert nodes together to build a list from a static set of inputs.

Once you fill in one connector, another optionla connector opens up. the node is still treated as fully connected in this case.

easy enough for built ins, we should allow user defined types to support this interface too someday. I guess in general, anything of the type

`fun :: a -> b -> b`

can be annotated as `@enable_daisy_chaining` " or whatever which enables this special interface which really just 

`... fun a1 . fun a2 . fun a3 $ b` or jsut `b` if there is no input

## boxed node internal connectors

boxed nodes have connectors on teh inside which wire its inputs/outputs to the implementation inside. The inputs can be labeled which determines the name of the variable in the function def/lambda.

when zoomed into a boxed node, teh internal input/output connectors go on the left/right edge of the screen 

### disconnected internal connectors

a disconnected internal input connector is an unused input when there is only one pattern match
i.e. `let f a b c = a + c` I guess in practice we should lint it to use holes `let f a _b c = a + c` or maybe `let f a _ c = a + c`

a disconnecte internal output connector is just a hole `let f a b c = _`


## export bindings connector

modules with no entry points can export stuff. To export you drag a value to an optional "export connector" on the right edge of the screen and the export connector also has a string entry field so you can name the exported value. if a let bound value was dragged in this way to an optional export connector (i.e. not an exsiting noe) then the let bound varibale name is used by default.





# connection

a connection is a line drawn from one "output" to one "input"

## loose connections

connections can just be dangling around as a inctermediary non-serialized state. In these cases, the loose ends cna be converted into a let binding or into a callsite node in the case of function types.

in code we can choose to 
- represent loose ends as unnamed let bindings i.e. `_ = ...`
- treat them as an invalid intermediary unserialized state that only exists on the node graph side of things

## node boundary interfacing connections (currying)
funcution definition nodes may have inputs and outputs within the node that connect to the nodes within the outer box node. 

## node boundary crossing input connections (captures)
function definition nodes may have connections permeate them
this forces the function definition node to be in the deepest scope of all captures, although in pracitce, the scope is determined by whatever scope one is in when the node got created and capturing from inaccesible scopes is forbidden by the UI.

QUESTION: do we allow node boundary crossing output connections? this is useful for boxing nodes just for the aske of organization

## monadic bind connection

see monadic context above

## recursion

in order to do recusion you must let bind the function into a variable, then using the let bound function inside the function looks like a capture

what does something like
`fix f = let x = f y, y = x in x` look like in the node graph?




# Additional Interfaces



## Application Runtime Inputs and Outputs

TODO

## Accessing stuff in the current scope

We need to create nodes from stuff avaliable in the scope, in particular:

- we need to be able to create callsite nodes from bulit in functions
- we also need to be able to create definition nodes to use as inputs into higher order functions
- we need to be able to fetch symbols from the current scope as well
- we should be able to copy existing globals/symbols 

to do this, lets have a single drop down search menu (and hotkey) that lets you search for stuff and then create bulit in and symboicated nodes out of them. 

## imports

there is some menu somewhere that lets you add / remove imported modules. imported modules are always sorted by name. tangle doesn't care about import qualifications, so all qualifications are treated the same in the viewer. In the reverse mapping, things are not qualified if they don't need to be, otherwise qualified, or we could do always fully qualified which might be easier to miplement.



# V2 features
## combined definition callsite nodes

combined nodes are a special treatement of definition nodes and callsite nodes combined into one. expressions representing nodes that can be combined are combined by defalut if conditions are met, or it can be disabled with `@no_combine` to enable this behavior. 

note you can also have a definition node tha is let bound to be combined as well. this is different than the callsite cmobined case as a let binding takes the function definition node as a higher order function input. A let bound combined function definition node has a label at the top naming teh variable it got let bound to. a let bound definition node can not be further combined with its callsite node though!

Here are the 3 cases of combined callsite nodes



1. If a unbound definition node has only one callsite, and the callsite is completely connected (all inputs connected)

normal case, this is basically making a lambda and appyling it to something. 

disconnected output is fine, it's just an unused output, could be rpresented in code as `let _ = \x -> ....`


2. if a unbound definition node has only one calliste, and it is only partially connected inputs

this is an intermediary state, it can be boxed again and converted into a function definition node from a curried function. 

we could consider treating this like a loose end of a non trivial let binding
i.e. `let _ = (\x y z -> ....) $ input1 _ input2` <- not real syntax, would be for intermediate state only if we chose to serialize invalid loose end states.

3. a partially connected combined node that got converted (boxed) into a curried funnctino definiton node

NO or maybe don't do this, it may be better to NOT do combined nodes in this case. Just force the explicit split definition node -> callsite node and then boxing the one partially connecetd callsite node into a curried function definition node. The cons of this approach is that you will need to do this split automatically for the user when they convert the partially conected combined node into a boxed function definition node and there may need to be special UI case for this.

