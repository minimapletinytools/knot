# TODO questions
- we currently use right click to access menu to delete
- should we support select on all features which shows a different ui for deleting and moving (in the case of boxed input connectors in focus mode)?
    - more mobile friendly
    - similarly to click and hold to edit icons on desktop

# type visual language

each type group gets a different graphic, there are condensed versions and expanded versions

- generic _ or a b c 
- typeclass a* b* c* maybe?
- primitive ℤ ℝ ℚ 𝔹 Σ [Σ] or 𝕊
- tuple ()
- list []
- map ↦
- custom ADT {}
- higher order function λ

# application
TODO

## search everything
"s" to search everywhere for adding new nodes.
TODO what can you search for

## scope search menu

allows you to search fo rstuff, has 2 view modes, list (colored rows) or nodes (grid display), after searching for anode you can drag it out into the canvas to use

### search bar
search bar is search bar, only searches for nodes that can be added.
symoblicated let bound variables can be searched by their names and also by their emoji spelled out and by the emoji itself

### filters
everything unselected by default, which gives a grey check box in everything and searches all
and then you can click on filter to filter for just that
- namespace filter
    - application functions filters
    - built in filter
    - imported namespace filter
- hierarchacal scope filter


# connection (rope)

## the line (rope) itself
### visual states
- type displayed on the line as a label
    - can be toggled on and off
- bind / normal (add some >> and/or use different line texture I guess)
    - defaults to normal style when dangling, and switches to bind style if connected monadically
- dangling / normal (no style needed here or grey out the line a bit)
- unravelling  (do a colored highlight)
    - unravelling highlights nodes and connections which an unravel goes through, hopefully it does't go too broad (really it should just land on 1 literal node in happy cases) but going deep is more so ok. 
- invalid (red or greyed out line)
- V2 trace forward (see which output objects are affected by a node)


### interaction modes
- delete
    - right-click > delete
- click + drag near line end disconnects it


## the connection (rope) ends

### visual states
- connected (no visual, line just dissapears into connector)
- dangling
    - type (matches input connector style)
    - normal / monadic / pattern match (monadic matches the monadic output connector style, pattern match matches pattern match input connector style)


### interaction modes
- click + drag to connect 

## hovering 
hovering over etiher the rope or its loose end shows full expression for rope type


# connectors (heads/tails of knots)

connectors exist on both boxed and non boxed nodes, they use the exact same visual language in both cases, except in the boxed case lines come out both sides, perhaps in the boxed case we need to do something different becasue we need space for the label somewhere?

## visual states
- connected/disconnected (required)
- invalid connection (maybe render a small X or no style needed as the line is styled)
    - upstream type changes 
- heads only
    - disconnected (optional)
    - nubbed (connector does not show at all)
    - pattern match connector
- tails only
    - multiplexable output (outputs for let bindings and definition nodes)
    - monadic output (only shows in monadic context, can be used as a non monadic output too of course, depends what it connects to)
    - record field outputs
- definition node output connector (shows at the bottom of a definition node)
- highlighted to indicate valid connection when dragging line end

## interactions modes 
- drag connector end to attach (types match)
- drag connector end and fail to attach (type mismatch)
- click + drag to detach connection from connected connector (does not work on let bound output connectors)
- click + drag to pull connection out of empty connector or multiplexed let bound output connector

## hovering 

same as hovering over rope, maybe shows some input/output conection specific info too?

## nubbed connections

nubbed inputs don't show the connector anymore, it's covered by the nub pill

### visual states

matches visula language of the connector but the connector is expanded into a pill, the type symbol stays on the left I guess

TODO figure out how to show label and value together in the nub

### interaction modes
- drag off to un nub
- drag a nubbable node onto the connector to nub
- right click > delete
- right click > un nub


### hovering

shows the connection type, and also shows the nubbed input value fully


# generic nodes

## visual states
- label 
- connectors (left side is input, right side is output, bottom side is definition)
- captured connections (maybe force all to come in from the top?)
- configuration dropdown somewhere in upper right maybe? or below the label 
    - not on all nodes but on many so make it a generic feature

### background color states
- nodes distinguish types based on background color as follows
- opaque: built in / callsite
    - white: regular
        - triangles: ADT update 
    - light green: literals
        - stripes: list
        - boxes: map
        - triangles: ADT
    - yellow: let binding
        - triangles: pattern matched ADT
    - light yellow: pattern match node
    - light red: bulit in math operators
    - red: bulit in control operators
    - pink: other common built in functions
    - goyard bg pattern: case/if expressions
- transparent: boxed node
    - blue: regular
    - green: entry point    
    - polkadot background: monadic
    - dotted outline: not focused
    - solid outline: focused


## interaction modes
- click / shift click to select
- click + drag on canvas to select
- drag to move
- select + backpsace to delete
- right click on label to access node menu (which probbaly has more than just delete in it?)
    - delete
    - auto arrange
    - pin location?

# let binding nodes and emoji nodes (symbol nodes)

let bindings have noe input and a multiplexed output, the node label name is the variable name. 

if the let binding is symoblicated to an emoji, the emoji covers the output node (you can still connect to it). 

then you can create a emoji somewhere in space (pulled from scope search menu) which is just an emoji with a mulitplexed output connector. I guess the emoji is also labeled with the let binding var name....



# inline pillbox nodes (slubs)

## visual states
- a pillbox shape, may be chained with lines between a longer pillbox 
- input/output labels omitted, only function name shows on slub
- perhaps a vertical represenattion is better for very long slubs, input at top let, output at bottom left
- TODO figure out slub style... they have both input and output... 

## interaction modes
- right click -> unslub (on slub boundary)
- right click -> unslub all
- right click (on slubbable node) -> slub

## hovering
shows full slub expression/types


# boxed nodes


## boxing nodes

after selecting several nodes, if boxing is allowed, you can right click -> box nodes

by default
- disconected inputs become box inputs
- connected inputs become captures
- If there is only one disconnected output connector it becomes the box output connector
- if there are several, the box output connector is a disconnected from the inside and you need to drag a connector to it.
- a definition output node shows up at the bottom of the boxed node

## focus mode 

boxed nodes can be focused in on, a focused node grey/blurs out stuff outside the node and enables some new intercations within the node. focusing a node dose not prevent you fro interacting with stuff outside of the focused node.

### interaciton modes
- double click or right click to focus
- escape or click outside to exit focus
- click drag box input connector disconnects it and converts it into a dangling connector, the box input connector remains and becomes a optional `_` sorta thing. 
- rigdht click > delete to delete box input connector
- click drag connection end to border to create box input connector, you can drag between input connectors for ordering
- reorder connectors 
    - I guess give a ≡ drag to reorder interface when focused on a node
    - or just disconnect + reconnect via the above operations, 
- right click > unbox node 
- right lick > hide/unhide node 
- you can drag nodes in and out of boxed nodes (focused or unfocused)


# node positioning 

TODO

adding nodes inside a boxed node will auto expand the boxed node box as needed