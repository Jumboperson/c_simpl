use clap::Parser;
use egg::{
    Analysis, DidMerge, EGraph, Extractor, Id, RecExpr, Rewrite, Runner, define_language, rewrite,
};
use std::fs;
use tree_sitter::Node;

fn print_tree(node: Node, source: &[u8], depth: usize, field_name: Option<&str>) {
    let indent = "  ".repeat(depth);

    // Format field name prefix if it exists (e.g., left: or operator:)
    let field_prefix = match field_name {
        Some(name) => format!("{}: ", name),
        None => "".to_string(),
    };

    // Check if the node is named and has no children (leaf node)
    let is_named = node.is_named();
    let node_text = node.utf8_text(source).unwrap_or("");

    if node.child_count() == 0 {
        println!(
            "{}{}[{}]: \"{}\"",
            indent,
            field_prefix,
            node.kind(),
            node_text
        );
    } else {
        println!("{}{}[{}]", indent, field_prefix, node.kind());

        // Iterate over all children, capturing their field names if present
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let child_field = node.field_name_for_child(child.id() as u32);
            print_tree(child, source, depth + 1, child_field);
        }
    }
}

define_language! {
    pub enum CExpr {
        "+" = Add([Id; 2]),
        "*" = Mul([Id; 2]),
        "-" = Sub([Id; 2]),
        "|" = Or([Id; 2]),
        "&" = And([Id; 2]),
        "<<" = Shl([Id; 2]),
        ">>" = Shr([Id; 2]),
        "~" = Not([Id; 1]),
        "^" = Xor([Id; 2]),
        "<" = LessThan([Id; 2]),
        ">" = GreaterThan([Id; 2]),
        // Represent function calls like bswap(x)
        "call" = Call([Id; 2]),
        "opaque" = Opaque([Id; 2]),
        Int(i64),
        Symbol(egg::Symbol),
    }
}

#[derive(Default)]
struct ConstantFolding;
impl Analysis<CExpr> for ConstantFolding {
    type Data = Option<i64>;

    fn merge(&mut self, to: &mut Self::Data, from: Self::Data) -> DidMerge {
        egg::merge_max(to, from)
    }

    fn make(egraph: &mut EGraph<CExpr, Self>, enode: &CExpr, _id: Id) -> Self::Data {
        let x = |i: &Id| egraph[*i].data;
        match enode {
            CExpr::Int(n) => Some(*n),
            CExpr::Add([a, b]) => Some(x(a)? + x(b)?),
            CExpr::Mul([a, b]) => Some(x(a)? * x(b)?),
            CExpr::Sub([a, b]) => Some(x(a)? - x(b)?),
            CExpr::Or([a, b]) => Some(x(a)? | x(b)?),
            CExpr::And([a, b]) => Some(x(a)? & x(b)?),
            CExpr::Shl([a, b]) => Some(x(a)? << x(b)?),
            CExpr::Shr([a, b]) => Some(x(a)? >> x(b)?),
            CExpr::Not([a]) => Some(!x(a)?),
            CExpr::Xor([a, b]) => Some(x(a)? ^ x(b)?),
            _ => None,
        }
    }

    fn modify(egraph: &mut EGraph<CExpr, Self>, id: Id) {
        if let Some(i) = egraph[id].data {
            let added = egraph.add(CExpr::Int(i));
            egraph.union(id, added);
        }
    }
}

fn parse_any_base(s: &str) -> Result<i64, std::num::ParseIntError> {
    // Trim whitespace first
    let s = s.trim();

    if let Some(hex) = s.strip_prefix("0x") {
        i64::from_str_radix(hex, 16)
    } else if let Some(bin) = s.strip_prefix("0b") {
        i64::from_str_radix(bin, 2)
    } else if let Some(oct) = s.strip_prefix("0o") {
        i64::from_str_radix(oct, 8)
    } else {
        // Fallback to standard base-10 decimal
        s.parse::<i64>()
    }
}

fn simplify_rules() -> Vec<Rewrite<CExpr, ConstantFolding>> {
    vec![
        // Basic algebraic rules
        rewrite!("add-comm"; "(+ ?a ?b)" => "(+ ?b ?a)"),
        rewrite!("mul-comm"; "(* ?a ?b)" => "(* ?b ?a)"),
        rewrite!("add-zero"; "(+ ?a 0)" => "?a"),
        rewrite!("mul-one"; "(* ?a 1)" => "?a"),
        rewrite!("mul-zero"; "(* ?a 0)" => "0"),
        rewrite!("and-comm"; "(& ?a ?b)" => "(& ?b ?a)"),
        rewrite!("or-comm"; "(| ?a ?b)" => "(| ?b ?a)"),
        rewrite!("or-reorder"; "(| ?a (| ?b ?c))" => "(| (| ?a ?b) ?c)"),
        rewrite!("lshift-and-dist"; "(<< (& ?a ?b) ?c)" => "(& (<< ?a ?c) (<< ?b ?c))"),
        rewrite!("rshift-and-dist"; "(>> (& ?a ?b) ?c)" => "(& (>> ?a ?c) (>> ?b ?c))"),
        // Byte-swap optimization pattern:
        // E.g., for 16-bit: ((x << 8) | (x >> 8)) -> bswap(x)
        rewrite!("bswap-16"; "(| (<< ?x 8) (>> ?x 8))" => "(call bswap16 ?x)"),
        // Explicit with &s
        rewrite!("bswap-32-expl";
            "(| (| (| (& (>> ?x 24) 255) (& (>> ?x 8) 65280)) (& (<< ?x 8) 16711680)) (& (<< ?x 24) 4278190080))"
            => "(call bswap32 ?x)"
        ),
        // Implicit for top and bottom bytes
        rewrite!("bswap-32-impl";
            "(| (| (| (>> ?x 24) (& (>> ?x 8) 65280)) (<< (& ?x 65280) 8)) (<< ?x 24))"
            => "(call bswap32 ?x)"
        ),
    ]
}

fn ast_to_egg_string(node: Node, source: &[u8]) -> String {
    match node.kind() {
        "identifier" => node.utf8_text(source).unwrap().to_string(),
        "number_literal" => {
            let string = node.utf8_text(source).unwrap();
            parse_any_base(string).unwrap().to_string()
        }
        "binary_expression" => {
            let op = node
                .child_by_field_name("operator")
                .unwrap()
                .utf8_text(source)
                .unwrap();
            let left = node.child_by_field_name("left").unwrap();
            let right = node.child_by_field_name("right").unwrap();
            format!(
                "({} {} {})",
                op,
                ast_to_egg_string(left, source),
                ast_to_egg_string(right, source)
            )
        }
        "parenthesized_expression" => {
            let inner = node.child(1).unwrap();
            ast_to_egg_string(inner, source)
        }
        _ => {
            if node.child_count() > 0 {
                // If it has children and we don't undertand it, treat it as an opaque
                format!("(opaque {} {})", node.start_byte(), node.end_byte())
            } else {
                // Otherwise its just another symbol
                node.utf8_text(source).unwrap().to_string()
            }
        }
    }
}

fn egg_expr_to_c(expr: &egg::RecExpr<CExpr>, id: egg::Id, source: &[u8]) -> String {
    let node = &expr[id];
    match node {
        CExpr::Symbol(s) => s.to_string(),
        CExpr::Int(i) => i.to_string(),
        CExpr::Add([l, r]) => format!(
            "({} + {})",
            egg_expr_to_c(expr, *l, source),
            egg_expr_to_c(expr, *r, source)
        ),
        CExpr::Mul([l, r]) => format!(
            "({} * {})",
            egg_expr_to_c(expr, *l, source),
            egg_expr_to_c(expr, *r, source)
        ),
        CExpr::Sub([l, r]) => format!(
            "({} - {})",
            egg_expr_to_c(expr, *l, source),
            egg_expr_to_c(expr, *r, source)
        ),
        CExpr::Or([l, r]) => format!(
            "({} | {})",
            egg_expr_to_c(expr, *l, source),
            egg_expr_to_c(expr, *r, source)
        ),
        CExpr::And([l, r]) => format!(
            "({} & {})",
            egg_expr_to_c(expr, *l, source),
            egg_expr_to_c(expr, *r, source)
        ),
        CExpr::Shl([l, r]) => format!(
            "({} << {})",
            egg_expr_to_c(expr, *l, source),
            egg_expr_to_c(expr, *r, source)
        ),
        CExpr::Shr([l, r]) => format!(
            "({} >> {})",
            egg_expr_to_c(expr, *l, source),
            egg_expr_to_c(expr, *r, source)
        ),
        CExpr::Not([l]) => format!("~{}", egg_expr_to_c(expr, *l, source)),
        CExpr::Xor([l, r]) => format!(
            "({} ^ {})",
            egg_expr_to_c(expr, *l, source),
            egg_expr_to_c(expr, *r, source)
        ),
        CExpr::LessThan([l, r]) => format!(
            "({} < {})",
            egg_expr_to_c(expr, *l, source),
            egg_expr_to_c(expr, *r, source)
        ),
        CExpr::GreaterThan([l, r]) => format!(
            "({} > {})",
            egg_expr_to_c(expr, *l, source),
            egg_expr_to_c(expr, *r, source)
        ),
        CExpr::Call([func, arg]) => format!(
            "{}({})",
            egg_expr_to_c(expr, *func, source),
            egg_expr_to_c(expr, *arg, source)
        ),
        CExpr::Opaque([start_id, end_id]) => {
            let starts = egg_expr_to_c(expr, *start_id, source);
            let ends = egg_expr_to_c(expr, *end_id, source);
            let start = parse_any_base(&starts).unwrap() as usize;
            let end = parse_any_base(&ends).unwrap() as usize;
            println!("{} {} {} {}", start, end, starts, ends);
            std::str::from_utf8(&source[start..end])
                .unwrap()
                .to_string()
        }
        _ => panic!("Unimplemented {:?}", node),
    }
}

fn simplify_node_text(node: Node, source: &[u8], replacements: &mut Vec<(usize, usize, String)>) {
    // Target both general binary expressions and bitwise combinations
    if node.kind() == "binary_expression" {
        let egg_str = ast_to_egg_string(node, source);
        //println!("Egg str: {:?}", &egg_str);
        if let Ok(expr) = egg_str.parse::<RecExpr<CExpr>>() {
            //println!("Unopt Expr: {:?}", &expr);
            let mut runner = Runner::<CExpr, ConstantFolding, ()>::default()
                .with_explanations_enabled()
                .with_expr(&expr)
                .run(&simplify_rules());
            let mut extractor = Extractor::new(&runner.egraph, egg::AstSize);
            let (_, best_expr) = extractor.find_best(runner.roots[0]);
            //println!("Opt Expr: {:?} Root Node: {:?} \nPretty Expr: {}", &best_expr, best_expr.root(), best_expr.pretty(32));

            let mut simplified_c = egg_expr_to_c(&best_expr, best_expr.root(), source);
            // Trim extraneous parentheses, jank asf
            if simplified_c.starts_with("(") && simplified_c.ends_with(")") {
                simplified_c = (&simplified_c[1..simplified_c.len() - 1]).to_string();
            }

            if expr.pretty(32) != best_expr.pretty(32) {
                println!("Found Optimization!");
                println!("Original C Expr: {}", node.utf8_text(source).unwrap());
                println!(
                    "Explanation:\n{}",
                    runner
                        .explain_equivalence(&expr, &best_expr)
                        .get_flat_string()
                );
                println!("Optimized C Version: {}\n", simplified_c);
            }

            if simplified_c != egg_str {
                replacements.push((node.start_byte(), node.end_byte(), simplified_c));
                return;
            }
        } else {
            println!("ERROR {:?}", egg_str.parse::<RecExpr<CExpr>>());
        }
    } else {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            simplify_node_text(child, source, replacements);
        }
    }
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Filename to try our simplification on
    filename: String,
}

fn main() {
    let args = Args::parse();
    // Code containing a manual 16-bit byteswap pattern
    let c_code = &fs::read_to_string(&args.filename).unwrap();

    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_c::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(c_code, None).unwrap();

    // Debug printing the C AST
    //print_tree(tree.root_node(), c_code.as_bytes(), 0, None);

    let mut replacements = Vec::new();
    simplify_node_text(tree.root_node(), c_code.as_bytes(), &mut replacements);

    replacements.sort_by(|a, b| b.0.cmp(&a.0));
    let mut modified_code = c_code.to_string();
    for (start, end, new_text) in replacements {
        if end - start > new_text.len() {
            modified_code.replace_range(start..end, &new_text);
        }
    }

    println!("Original C Code:\n{}", c_code);
    println!("Simplified C Code:\n{}", modified_code);
}
