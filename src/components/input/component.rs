use dioxus::prelude::*;

#[component]
pub fn Input(
    oninput: Option<EventHandler<FormEvent>>,
    onblur: Option<EventHandler<FocusEvent>>,
    onkeydown: Option<EventHandler<KeyboardEvent>>,
    #[props(extends = GlobalAttributes)]
    #[props(extends = input)]
    attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    rsx! {
        input {
            class: "input",
            oninput: move |e| _ = oninput.map(|cb| cb(e)),
            onblur: move |e| _ = onblur.map(|cb| cb(e)),
            onkeydown: move |e| _ = onkeydown.map(|cb| cb(e)),
            ..attributes,
            {children}
        }
    }
}
