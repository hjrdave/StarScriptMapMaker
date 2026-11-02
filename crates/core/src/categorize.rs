//! Gives the Layers of a Map from before categories one (ADR 0013).

use crate::map::Subtree;
use crate::node::{Category, Node};

/// Gives every top-level Layer, and the Layers nested in it, a category, and puts the top-level
/// Layers in the fixed category order. Used when a file or template older than format 1.12 is
/// read, since its Layers carry none.
///
/// A Layer takes the category of the Objects inside it (and inside the Layers nested in it).
/// One that holds more than one kind goes to Land, keeping its Objects. An empty one takes the
/// category named by its own name, "Labels", "Assets", "Land" or "Backdrop", else Land. Within
/// a category the Stacking order is kept.
pub(crate) fn categorize(layers: &mut [Subtree]) {
    for top in layers.iter_mut() {
        let mut found = Vec::new();
        object_categories(top, &mut found);
        let category = match found.as_slice() {
            [] => named(top).unwrap_or(Category::Land),
            [first, rest @ ..] if rest.iter().all(|c| c == first) => *first,
            _ => Category::Land,
        };
        assign(top, category);
    }
    // Lowest first: Backdrop, then Land, Assets and Labels on top.
    layers.sort_by_key(|top| match top.node {
        Node::Layer(ref layer) => std::cmp::Reverse(layer.category),
        Node::Object(_) => std::cmp::Reverse(Category::Land),
    });
}

fn object_categories(tree: &Subtree, found: &mut Vec<Category>) {
    for child in &tree.children {
        match &child.node {
            Node::Object(object) => found.push(object.kind.category()),
            Node::Layer(_) => object_categories(child, found),
        }
    }
}

fn named(tree: &Subtree) -> Option<Category> {
    let Node::Layer(layer) = &tree.node else {
        return None;
    };
    let name = layer.name.trim();
    Category::ALL
        .into_iter()
        .find(|category| category.name().eq_ignore_ascii_case(name))
}

fn assign(tree: &mut Subtree, category: Category) {
    if let Node::Layer(layer) = &mut tree.node {
        layer.category = category;
    }
    for child in &mut tree.children {
        assign(child, category);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Id;
    use crate::node::{Layer, Object, ObjectKind, Stamp};

    fn layer(name: &str, children: Vec<Subtree>) -> Subtree {
        Subtree {
            id: Id::new(),
            node: Node::Layer(Layer::new(name)),
            children,
        }
    }

    fn stamp() -> Subtree {
        Subtree {
            id: Id::new(),
            node: Node::Object(Object::new(ObjectKind::Stamp(Stamp::default()))),
            children: Vec::new(),
        }
    }

    fn label() -> Subtree {
        Subtree {
            id: Id::new(),
            node: Node::Object(Object::new(ObjectKind::Label(Box::default()))),
            children: Vec::new(),
        }
    }

    fn category(tree: &Subtree) -> Category {
        match &tree.node {
            Node::Layer(layer) => layer.category,
            Node::Object(_) => unreachable!("a Layer"),
        }
    }

    #[test]
    fn a_layer_takes_the_kind_it_holds_and_a_mixed_one_goes_to_land() {
        let mut layers = vec![
            layer("Trees", vec![stamp(), stamp()]),
            layer("Names", vec![label()]),
            layer("Both", vec![stamp(), label()]),
        ];
        categorize(&mut layers);
        let by_name = |name: &str| {
            layers
                .iter()
                .find(|l| matches!(&l.node, Node::Layer(x) if x.name == name))
                .map(category)
        };
        assert_eq!(by_name("Trees"), Some(Category::Assets));
        assert_eq!(by_name("Names"), Some(Category::Labels));
        assert_eq!(by_name("Both"), Some(Category::Land));
    }

    #[test]
    fn an_empty_layer_is_named_for_its_category_or_is_land() {
        let mut layers = vec![
            layer("labels", Vec::new()),
            layer("Assets", Vec::new()),
            layer("Backdrop", Vec::new()),
            layer("Mountains", Vec::new()),
        ];
        categorize(&mut layers);
        let all: Vec<_> = layers.iter().map(category).collect();
        assert!(all.contains(&Category::Labels));
        assert!(all.contains(&Category::Assets));
        assert!(all.contains(&Category::Backdrop));
        assert!(all.contains(&Category::Land));
    }

    #[test]
    fn nested_layers_take_the_top_layers_category_and_the_order_is_fixed() {
        // Stamps in a Group, labels below them, a Land layer on top: the fixed order has the
        // Labels on top, then Assets, then Land, with Backdrop lowest.
        let mut layers = vec![
            layer("Backdrop", Vec::new()),
            layer("Names", vec![label()]),
            layer("Forest", vec![layer("Pines", vec![stamp()])]),
            layer("Mountains", Vec::new()),
        ];
        categorize(&mut layers);
        let order: Vec<_> = layers.iter().map(category).collect();
        assert_eq!(
            order,
            [
                Category::Backdrop,
                Category::Land,
                Category::Assets,
                Category::Labels
            ]
        );
        let forest = layers
            .iter()
            .find(|l| category(l) == Category::Assets)
            .unwrap();
        assert_eq!(category(&forest.children[0]), Category::Assets);
    }

    #[test]
    fn layers_of_one_category_keep_their_order() {
        let mut layers = vec![
            layer("B", vec![stamp()]),
            layer("A", vec![stamp()]),
            layer("C", vec![stamp()]),
        ];
        categorize(&mut layers);
        let names: Vec<_> = layers
            .iter()
            .filter_map(|l| match &l.node {
                Node::Layer(x) => Some(x.name.as_str()),
                Node::Object(_) => None,
            })
            .collect();
        assert_eq!(names, ["B", "A", "C"]);
    }
}
