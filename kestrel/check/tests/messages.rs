//! The help that comes with an error says what to do: a missing struct, an enum or a constant
//! called like a function, and a vector given only some of its components.

use kestrel_check::check;
use kestrel_syntax::parse;

/// The message and help of each error in `src`.
fn errors(src: &str) -> Vec<(String, Option<String>)> {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    check(&p).1.into_iter().map(|d| (d.message, d.help)).collect()
}

fn one(msg: &str, help: &str) -> Vec<(String, Option<String>)> {
    vec![(msg.to_string(), Some(help.to_string()))]
}

const DECLARE_BALL: &str = "declare it at the top level of a file (`struct Ball { ... }`), or bring it in from a module with `use { Ball from m }`";

#[test]
fn an_unknown_capitalised_call_is_an_unknown_struct() {
    assert_eq!(errors("main() {\n let b = Ball(1)\n}\n"), one("unknown struct `Ball`", DECLARE_BALL));
    // A lowercase name is still a function.
    assert_eq!(errors("main() {\n let b = ball(1)\n}\n"), vec![("unknown function `ball`".to_string(), None)]);
    // The same in a constant and in a field default, which take struct values.
    assert_eq!(errors("let X = Ball(1)\nmain() {}\n"), one("unknown struct `Ball`", DECLARE_BALL));
    let colour = "declare it at the top level of a file (`struct Colour { ... }`), or bring it in from a module with `use { Colour from m }`";
    assert_eq!(errors("struct P {\n c: int = Colour(1, 0, 0)\n}\nmain() {}\n"), one("unknown struct `Colour`", colour));
}

#[test]
fn an_enum_called_like_a_struct_says_how_to_build_one() {
    let src = "enum State {\n Idle\n Run\n}\nmain() {\n let s = State()\n}\n";
    assert_eq!(
        errors(src),
        one("`State` is an enum, so it can't be called like a struct", "build a value from one of its variants, like `State.Idle`")
    );
    // A variant with fields is built with arguments.
    let shape = "enum Shape {\n Circle { r: float }\n Dot\n}\nlet S = Shape()\nmain() {}\n";
    assert_eq!(
        errors(shape),
        one("`Shape` is an enum, so it can't be called like a struct", "build a value from one of its variants, like `Shape.Circle(...)`")
    );
}

#[test]
fn a_constant_called_like_a_function_says_to_drop_the_parens() {
    let help = "use it without `()`: `RED`";
    assert_eq!(errors("let RED = 1\nmain() {\n print(RED())\n}\n"), one("`RED` is a constant, not a function", help));
    assert_eq!(errors("let RED = 1\nstruct P {\n c: int = RED()\n}\nmain() {}\n"), one("`RED` is a constant, not a function", help));
    // A constant declared after the one that calls it.
    assert_eq!(errors("let X = RED()\nlet RED = 1\nmain() {}\n"), one("`RED` is a constant, not a function", help));
}

#[test]
fn a_vector_needs_every_component_or_none() {
    assert_eq!(
        errors("main() {\n let v = vec3(1)\n}\n"),
        one("missing fields `y` and `z` for `vec3`", "give every component, or none for a zero vector: `vec3()`")
    );
    assert_eq!(
        errors("main() {\n let v = vec2(x: 1)\n}\n"),
        one("missing field `y` for `vec2`", "give every component, or none for a zero vector: `vec2()`")
    );
    assert_eq!(errors("let O = vec3()\nmain() {\n let v = vec2()\n print(O, v)\n}\n"), vec![]);
}
