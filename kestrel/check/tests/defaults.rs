//! Field defaults and constants may be struct and enum values; defaults that build each other
//! forever are an error, and so is a default that builds a value owning an arena or GPU object.

use kestrel_check::check;
use kestrel_syntax::parse;

const HEAD: &str = "enum State {\n Idle\n Run\n}\nenum Shape {\n Dot\n Circle { r: float }\n}\nstruct Color {\n r: float\n g: float\n b: float\n}\nlet RED = Color(1, 0, 0)\nlet START = State.Idle\n";

fn errors(body: &str) -> Vec<String> {
    let src = format!("{HEAD}{body}");
    let (p, d) = parse(&src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    check(&p).1.into_iter().map(|d| d.message).collect()
}

fn none() -> Vec<String> {
    Vec::new()
}

const LOOP_HELP: &str = "; set the field when you build it, or start it empty (`[]`)";

#[test]
fn defaults_and_constants_may_be_struct_and_enum_values() {
    let body = "struct Player {\n tint: Color = RED\n state: State = START\n shape: Shape = Shape.Circle(2)\n mode: State = State.Run\n dir: vec2 = vec2(0, 1)\n}\nlet ORDER = [State.Idle, State.Run]\nmain() {\n print(Player(), ORDER)\n}\n";
    assert_eq!(errors(body), none());
}

#[test]
fn a_default_may_build_a_struct_that_holds_heap_memory() {
    let body = "struct Inventory {\n items: list[int] = []\n}\nstruct Player {\n inv: Inventory = Inventory()\n names: [string] = [\"a\"]\n}\nmain() {\n print(Player())\n}\n";
    assert_eq!(errors(body), none());
}

#[test]
fn a_default_cant_build_an_owning_value_except_an_arena() {
    assert_eq!(errors("struct Pool {\n mem: Arena = Arena()\n}\nmain() {}\n"), none());
    assert_eq!(
        errors("struct Pool {\n mem: Arena = Arena()\n}\nstruct World {\n pool: Pool = Pool()\n}\nmain() {}\n"),
        vec!["field defaults can't build a value that owns an `Arena` or a GPU object; set the field when you build it".to_string()]
    );
}

#[test]
fn defaults_that_build_each_other_forever_are_an_error() {
    assert_eq!(
        errors("struct Node {\n kids: list[Node] = [Node()]\n}\nmain() {}\n"),
        vec![format!("field defaults build each other in a loop: Node.kids -> Node.kids{LOOP_HELP}")]
    );
    assert_eq!(
        errors("struct Tree {\n kids: [Tree] = KIDS\n}\nlet KIDS = [Tree()]\nmain() {}\n"),
        vec![format!("field defaults build each other in a loop: Tree.kids -> KIDS -> Tree.kids{LOOP_HELP}")]
    );
    assert_eq!(
        errors("struct A {\n bs: list[B] = [B()]\n}\nstruct B {\n items: list[A] = [A()]\n}\nmain() {}\n"),
        vec![format!("field defaults build each other in a loop: A.bs -> B.items -> A.bs{LOOP_HELP}")]
    );
    // A field given at the construction isn't built from its default.
    assert_eq!(errors("struct Node {\n kids: list[Node] = [Node(kids: [])]\n}\nmain() {}\n"), none());
}

#[test]
fn defaults_must_be_constant_expressions() {
    assert_eq!(
        errors("f() -> int {\n 1\n}\nstruct P {\n x: int = f()\n}\nmain() {}\n"),
        vec!["field defaults can only use literals, operators, constants, struct and enum values, and pure built-ins like `sqrt`".to_string()]
    );
    assert_eq!(
        errors("f() -> int {\n 1\n}\nlet X = f()\nmain() {}\n"),
        vec!["constants can only use literals, operators, other constants, struct and enum values, and pure built-ins like `sqrt`".to_string()]
    );
}

#[test]
fn a_string_optional_default_from_a_constant_points_at_no_arena() {
    let body = "let NAME = \"n\"\nstruct S {\n name: string? = NAME\n}\nmain() {\n var keep = S()\n arena {\n  keep = S()\n }\n print(keep)\n}\n";
    assert_eq!(errors(body), none());
}

#[test]
fn a_constant_that_is_an_arena_may_be_a_default() {
    assert_eq!(errors("let A = Arena()\nstruct Pool {\n mem: Arena = A\n}\nmain() {}\n"), none());
    // A constant that holds an arena inside a struct still builds an owning value.
    assert_eq!(
        errors("struct Pool {\n mem: Arena = Arena()\n}\nlet P = Pool()\nstruct World {\n pool: Pool = P\n}\nmain() {}\n"),
        vec!["field defaults can't build a value that owns an `Arena` or a GPU object; set the field when you build it".to_string()]
    );
}

#[test]
fn a_loop_through_the_same_default_twice_is_reported_once() {
    assert_eq!(
        errors("struct Node {\n kids: [Node] = [Node(), Node()]\n}\nmain() {}\n"),
        vec![format!("field defaults build each other in a loop: Node.kids -> Node.kids{LOOP_HELP}")]
    );
    // Two fields that each close a loop through the same constant are two loops, each reported once.
    assert_eq!(
        errors("struct Tree {\n kids: [Tree] = KIDS\n more: [Tree] = KIDS\n}\nlet KIDS = [Tree(), Tree()]\nmain() {}\n"),
        vec![
            format!("field defaults build each other in a loop: Tree.kids -> KIDS -> Tree.kids{LOOP_HELP}"),
            format!("field defaults build each other in a loop: KIDS -> Tree.more -> KIDS{LOOP_HELP}"),
        ]
    );
}
