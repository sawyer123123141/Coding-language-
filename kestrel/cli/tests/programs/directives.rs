//! The tests of the `// expect...`, `// env:` and `// asset:` directives a test program declares.

use super::expectations;

#[test]
#[should_panic(expected = "typo.kes: unknown expect directive: // expect-stderr missing colon")]
fn unknown_expect_directive_panics() {
    expectations("typo.kes", "// expect-stderr missing colon\nmain() {}\n");
}

#[test]
#[should_panic(expected = "typo.kes: bad expect-exit number: // expect-exit: one")]
fn bad_expect_exit_number_panics() {
    expectations("typo.kes", "// expect-exit: one\nmain() {}\n");
}

#[test]
fn the_proof_check_run_can_expect_another_failure() {
    let exp = expectations("p.kes", "// expect-exit: 101\n// expect-stderr: division by zero\n// expect-stderr-proofs: assumption `b > 0` is false\nmain() {}\n");
    assert_eq!((exp.exit, exp.stderr.clone()), (101, vec!["division by zero".to_string()]));
    let proofs = exp.with_proofs();
    assert_eq!((proofs.exit, proofs.stderr), (101, vec!["assumption `b > 0` is false".to_string()]));
    let exp = expectations("p.kes", "// expect: hi\n// expect-exit-proofs: 101\nmain() {}\n");
    assert_eq!((exp.exit, exp.with_proofs().exit, exp.with_proofs().stdout), (0, 101, "hi\n".to_string()));
    // Without them, the run with the proof check expects what the plain run does.
    let exp = expectations("p.kes", "// expect-exit: 3\n// expect-stderr: no\nmain() {}\n");
    assert_eq!((exp.with_proofs().exit, exp.with_proofs().stderr), (3, vec!["no".to_string()]));
}

#[test]
#[should_panic(expected = "p.kes: bad expect-exit-proofs number: // expect-exit-proofs: x")]
fn bad_expect_exit_proofs_number_panics() {
    expectations("p.kes", "// expect-exit-proofs: x\nmain() {}\n");
}

#[test]
fn env_directives_are_collected() {
    let exp = expectations("env.kes", "// env: A=1\n// env:  B = two words \n// env: C=\nmain() {}\n");
    let expected = [("A", "1"), ("B", "two words"), ("C", "")];
    assert_eq!(exp.env, expected.map(|(k, v)| (k.to_string(), v.to_string())));
}

#[test]
fn asset_directives_are_collected() {
    let exp = expectations("asset.kes", "// asset: a.png\n// asset:  sub/b.bin \n// expect: ok\nmain() {}\n");
    assert_eq!(exp.assets, ["a.png", "sub/b.bin"]);
    assert_eq!(exp.stdout, "ok\n", "and the other directives still count");
    assert!(expectations("none.kes", "main() {}\n").assets.is_empty());
}

#[test]
#[should_panic(expected = "typo.kes: bad asset directive (expected a file inside tests/programs): // asset:")]
fn an_asset_directive_without_a_file_panics() {
    expectations("typo.kes", "// asset:\nmain() {}\n");
}

#[test]
#[should_panic(expected = "typo.kes: bad asset directive (expected a file inside tests/programs): // asset: ../outside.png")]
fn an_asset_outside_the_programs_folder_panics() {
    expectations("typo.kes", "// asset: ../outside.png\nmain() {}\n");
}

#[test]
#[should_panic(expected = "typo.kes: unknown expect directive: // expect-asset: a.png")]
fn an_unknown_directive_still_panics() {
    expectations("typo.kes", "// expect-asset: a.png\nmain() {}\n");
}

#[test]
#[should_panic(expected = "e.kes: `// copy:` only works in the voxel engine's tests")]
fn the_engine_copy_directive_is_refused_here() {
    expectations("e.kes", "// copy: kestrel/voxel/grid.kes\nmain() {}\n");
}

#[test]
#[should_panic(expected = "e.kes: `// replace:` only works in the voxel engine's tests")]
fn the_engine_replace_directive_is_refused_here() {
    expectations("e.kes", "// replace: a.kes x => y\nmain() {}\n");
}

#[test]
#[should_panic(expected = "e.kes: `// release` only works in the voxel engine's tests")]
fn the_engine_release_directive_is_refused_here() {
    expectations("e.kes", "// release\nmain() {}\n");
}

#[test]
#[should_panic(expected = "e.kes: `// only-jit` only works in the voxel engine's tests")]
fn the_engine_only_jit_directive_is_refused_here() {
    expectations("e.kes", "// only-jit\nmain() {}\n");
}

#[test]
fn a_comment_that_starts_with_the_word_release_is_not_a_directive() {
    expectations("note.kes", "// release pass runs this with the proof check on\n// only-jit-ish\nmain() {}\n");
}

#[test]
#[should_panic(expected = "e.kes: unknown directive `// relase`; did you mean `// release`?")]
fn a_misspelt_release_panics() {
    expectations("e.kes", "// relase\nmain() {}\n");
}

#[test]
#[should_panic(expected = "e.kes: unknown directive `// onlyjit`; did you mean `// only-jit`?")]
fn a_misspelt_only_jit_panics() {
    expectations("e.kes", "// onlyjit\nmain() {}\n");
}

#[test]
#[should_panic(expected = "e.kes: unknown directive `// replce: a.kes x => y`; did you mean `// replace:`?")]
fn a_misspelt_replace_panics() {
    expectations("e.kes", "// replce: a.kes x => y\nmain() {}\n");
}

#[test]
fn ordinary_comments_are_not_near_misses() {
    expectations("note.kes", "// relaxed\n// released into the wild\n// note: see above\n// replaced\nmain() {}\n");
}
