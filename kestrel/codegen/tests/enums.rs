mod common;

use common::{call0, call1, jit};

const ENEMIES: &str = "enum Enemy {
    Zombie { hp: int, speed: float }
    Skeleton { hp: int, arrows: i64 }
    Ghost

    take_damage(var self, dmg: int) {
        match self {
            Zombie z { z.hp -= dmg }
            Skeleton s { s.hp -= dmg * 2 }
            Ghost { }
        }
    }

    pure hp(self) -> int {
        match self {
            Zombie z { z.hp }
            Skeleton s { s.hp }
            Ghost { 0 }
        }
    }
}

struct Squad {
    leader: Enemy
    size: int
}

make(n: int) -> Enemy {
    Enemy.Zombie(hp: n, speed: 0.5)
}

t_damage() -> int {
    var z = Enemy.Zombie(hp: 10, speed: 1.5)
    var s = Enemy.Skeleton(arrows: 3, hp: 20)
    z.take_damage(3)
    s.take_damage(3)
    z.hp() * 100 + s.hp()
}

t_copy() -> int {
    var a = Enemy.Skeleton(hp: 5, arrows: 9)
    var b = a
    b.take_damage(1)
    a.hp() * 10 + b.hp()
}

t_tag(n: int) -> int {
    let e = if n > 0 { Enemy.Ghost } else { Enemy.Zombie(1, 2.0) }
    match e {
        Ghost { 1 }
        else { 2 }
    }
}

t_nested() -> i64 {
    var sq = Squad(Enemy.Skeleton(hp: 1, arrows: 40000000000), 3)
    sq.leader.take_damage(0)
    match sq.leader {
        Skeleton s { s.arrows + i64(sq.size) }
        else { 0 }
    }
}

t_rvalue() -> int {
    match make(4) {
        Zombie z { z.hp }
        else { -1 }
    }
}
";

#[test]
fn bindings_change_the_matched_value_in_place() {
    let p = jit(ENEMIES);
    assert_eq!(call0::<i32>(&p, "t_damage"), 714);
}

#[test]
fn enum_copies_are_independent() {
    let p = jit(ENEMIES);
    assert_eq!(call0::<i32>(&p, "t_copy"), 53);
}

#[test]
fn match_tests_the_tag() {
    let p = jit(ENEMIES);
    assert_eq!(call1(&p, "t_tag", 1), 1);
    assert_eq!(call1(&p, "t_tag", -1), 2);
    assert_eq!(call0::<i32>(&p, "t_rvalue"), 4);
}

#[test]
fn enums_inside_structs() {
    let p = jit(ENEMIES);
    assert_eq!(call0::<i64>(&p, "t_nested"), 40_000_000_003);
}
