
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
    - multiplexed output (looks the same, but worth calling out)
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
- connectors 

## interaction modes
- click / shift click to select
- click + drag on canvas to select
- drag to move



# boxed nodes


## boxing nodes

after selecting several nodes, if boxing is allowed, you can right click -> box nodes

by default
- disconected inputs become box inputs
- connected inputs become captures
- If there is only one disconnected output connector it becomes the box output connector
- if there are several, the box output connector is a disconnected from the inside and you need to drag a connector to it.

## focus mode 

boxed nodes can be focused in on, a focused node grey/blurs out stuff outside the node and enables some new intercations within the node

### interaciton modes
- double click or right click to focus
- escape or click outside to exit focus
- click drag box input connector disconnects it and converts it into a dangling connector, the box input connector remains and becomes a optional `_` sorta thing. 
- rigdht click > delete to delete box input connector
- click drag connection end to border to create box input connector, you can drag between input connectors for ordering
- reorder connectors 
    - I guess give a ≡ drag to reorder interface when focused on a node
    - or just disconnect + reconnect via the above operations, 