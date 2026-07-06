Lisp / S-expression syntax
Comments
Metadata annotations for graph layout, node IDs, docs
Modules/imports
Top-level definitions
Function definitions
Type signatures



Pure functional
Call-by-need / lazy evaluation
Immutable values
Stable semantic hashing for caching (I don't think this is needed)
Explicit recursion support


Primitive types:
  Bool, Int, Float, String, Unit

Unit-aware numeric types:
  Length, Speed, Temperature, Percent, Angle, Time, etc.

Composite types:
  Records
  Enums / tagged unions
  Function types
  Built-in generic containers

Containers:
    List<T>
    Map<K, V>
    Option<T>
    Result<T, E>

Literals
Variables
Function application
let bindings
lambda / fn
if
match
record construction
record access
record update
enum/variant construction
list/map construction

built-in Monads:
    IO<T>
    Option<T>
    Result<T, E>

built-in interfaces:
    monoid
    semigroup
    Eq
    Ord
    Show
    Functor
    Folbable
    Semigroup
    Monoid
    MAYBE Monad if possible?
    (skip Monad, travesible, applicative)

Alternative intefaces:
    Collection
        map
        foldl
        foldr
        filter
        length

    Mergeable
        empty
        merge

    Context
        bind
        pure