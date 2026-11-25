//! An XML text kept as it came: every element, attribute, comment and run of white space, so that
//! writing an unchanged document gives the bytes it was read from.

/// How deep elements may nest: a text nested deeper is refused before it exhausts the stack.
const MAX_DEPTH: usize = 64;

/// Where a text stops being XML this reads, as a byte offset, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Malformed {
    pub at: usize,
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Attribute {
    /// The white space before the name.
    pub lead: String,
    pub name: String,
    /// The `=` and the white space around it.
    pub equals: String,
    pub quote: char,
    /// The text between the quotes, its entities unread.
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Node {
    Element(Element),
    /// Text, a comment, a CDATA section or a processing instruction, as it came.
    Other(String),
}

impl Node {
    /// Whether this is text of white space alone.
    pub fn is_space(&self) -> bool {
        self.space().is_some()
    }

    /// The text of this, where it is white space alone.
    pub fn space(&self) -> Option<&str> {
        match self {
            Self::Other(text) if text.chars().all(char::is_whitespace) => Some(text),
            _ => None,
        }
    }
}

/// What stands between an element's tags.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Body {
    pub nodes: Vec<Node>,
    /// The white space after the name in the closing tag.
    pub close_tail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Element {
    pub name: String,
    pub attributes: Vec<Attribute>,
    /// The white space between the last attribute and the end of the opening tag.
    pub tail: String,
    /// `None` for an element closed in its own tag.
    pub body: Option<Body>,
}

impl Element {
    /// An element closed in its own tag that states `value`, a space before its slash.
    pub fn leaf(name: &str, value: String) -> Self {
        let mut leaf = Self {
            name: name.to_string(),
            tail: " ".to_string(),
            ..Self::default()
        };
        leaf.set("value", value);
        leaf
    }

    /// The text of attribute `name`, as it came.
    pub fn attribute(&self, name: &str) -> Option<&str> {
        (self.attributes.iter())
            .find(|a| a.name == name)
            .map(|a| a.value.as_str())
    }

    /// State `value`, which holds no quote, for attribute `name`, in place where the element has one.
    pub fn set(&mut self, name: &str, value: String) {
        match self.attributes.iter_mut().find(|a| a.name == name) {
            Some(attribute) => attribute.value = value,
            None => self.attributes.push(Attribute {
                lead: " ".to_string(),
                name: name.to_string(),
                equals: "=".to_string(),
                quote: '"',
                value,
            }),
        }
    }

    /// The child elements, in order.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        (self.body.iter())
            .flat_map(|body| &body.nodes)
            .filter_map(|node| match node {
                Node::Element(element) => Some(element),
                Node::Other(_) => None,
            })
    }

    /// The first child element named `name`.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|element| element.name == name)
    }

    pub fn write(&self, out: &mut String) {
        out.push('<');
        out.push_str(&self.name);
        for a in &self.attributes {
            out.push_str(&a.lead);
            out.push_str(&a.name);
            out.push_str(&a.equals);
            out.push(a.quote);
            out.push_str(&a.value);
            out.push(a.quote);
        }
        out.push_str(&self.tail);
        let Some(body) = &self.body else {
            out.push_str("/>");
            return;
        };
        out.push('>');
        for node in &body.nodes {
            match node {
                Node::Element(element) => element.write(out),
                Node::Other(text) => out.push_str(text),
            }
        }
        out.push_str("</");
        out.push_str(&self.name);
        out.push_str(&body.close_tail);
        out.push('>');
    }
}

/// A document: what stands before its root, the root, and what stands after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Document {
    pub prolog: String,
    pub root: Element,
    pub epilog: String,
}

impl Document {
    pub fn parse(text: &str) -> Result<Self, Malformed> {
        let mut scan = Scan { text, at: 0 };
        scan.misc()?;
        let prolog = text[..scan.at].to_string();
        if !scan.rest().starts_with('<') {
            return Err(scan.fault("no root element"));
        }
        let root = scan.element(0)?;
        let after = scan.at;
        scan.misc()?;
        if !scan.rest().is_empty() {
            return Err(scan.fault("text after the root element"));
        }
        Ok(Self {
            prolog,
            root,
            epilog: text[after..].to_string(),
        })
    }
}

struct Scan<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Scan<'a> {
    fn rest(&self) -> &'a str {
        &self.text[self.at..]
    }

    fn fault(&self, why: impl Into<String>) -> Malformed {
        Malformed {
            at: self.at,
            why: why.into(),
        }
    }

    /// Take the text up to and including `end`.
    fn through(&mut self, end: &str, what: &str) -> Result<&'a str, Malformed> {
        let rest = self.rest();
        let found = rest
            .find(end)
            .ok_or_else(|| self.fault(format!("{what} that does not end")))?;
        self.at += found + end.len();
        Ok(&rest[..found + end.len()])
    }

    /// Take the run of characters `keep` holds for.
    fn run(&mut self, keep: impl Fn(char) -> bool) -> &'a str {
        let rest = self.rest();
        let end = rest.find(|c| !keep(c)).unwrap_or(rest.len());
        self.at += end;
        &rest[..end]
    }

    /// Pass the white space, comments and processing instructions that may stand outside the root.
    fn misc(&mut self) -> Result<(), Malformed> {
        loop {
            // A byte order mark may lead the text.
            self.run(|c| c.is_whitespace() || c == '\u{feff}');
            let rest = self.rest();
            if rest.starts_with("<?") {
                self.through("?>", "a processing instruction")?;
            } else if rest.starts_with("<!--") {
                self.through("-->", "a comment")?;
            } else if rest.starts_with("<!") {
                return Err(self.fault("a document type declaration, which is not read"));
            } else {
                return Ok(());
            }
        }
    }

    /// Read the element whose `<` the scan stands on.
    fn element(&mut self, depth: usize) -> Result<Element, Malformed> {
        if depth > MAX_DEPTH {
            return Err(self.fault(format!("elements nested deeper than {MAX_DEPTH}")));
        }
        self.at += 1;
        let name = self.run(is_name).to_string();
        if name.is_empty() {
            return Err(self.fault("a tag with no name"));
        }
        if name.starts_with(['!', '?']) {
            return Err(self.fault("a declaration, which is not read"));
        }
        let mut attributes = Vec::new();
        let tail = loop {
            let lead = self.run(char::is_whitespace);
            let rest = self.rest();
            if rest.starts_with("/>") {
                self.at += 2;
                return Ok(Element {
                    name,
                    attributes,
                    tail: lead.to_string(),
                    body: None,
                });
            }
            if rest.starts_with('>') {
                self.at += 1;
                break lead.to_string();
            }
            let attribute = self.run(is_name);
            if attribute.is_empty() || lead.is_empty() {
                return Err(self.fault(format!("the tag of <{name}> does not end")));
            }
            let from = self.at;
            self.run(char::is_whitespace);
            if !self.rest().starts_with('=') {
                return Err(self.fault(format!("attribute {attribute} of <{name}> has no value")));
            }
            self.at += 1;
            self.run(char::is_whitespace);
            let equals = &self.text[from..self.at];
            let quote = (self.rest().chars().next())
                .filter(|c| matches!(c, '"' | '\''))
                .ok_or_else(|| {
                    self.fault(format!("attribute {attribute} of <{name}> is not quoted"))
                })?;
            self.at += 1;
            let value = self.run(|c| c != quote);
            if self.rest().is_empty() {
                return Err(self.fault(format!("attribute {attribute} of <{name}> does not end")));
            }
            self.at += 1;
            if attributes.iter().any(|a: &Attribute| a.name == attribute) {
                return Err(
                    self.fault(format!("attribute {attribute} of <{name}> is stated twice"))
                );
            }
            attributes.push(Attribute {
                lead: lead.to_string(),
                name: attribute.to_string(),
                equals: equals.to_string(),
                quote,
                value: value.to_string(),
            });
        };
        let mut nodes = Vec::new();
        loop {
            let rest = self.rest();
            if let Some(close) = rest.strip_prefix("</") {
                if !close.starts_with(name.as_str()) || close[name.len()..].starts_with(is_name) {
                    return Err(self.fault(format!("<{name}> is closed by another tag")));
                }
                self.at += 2 + name.len();
                let close_tail = self.run(char::is_whitespace).to_string();
                if !self.rest().starts_with('>') {
                    return Err(self.fault(format!("the closing tag of <{name}> does not end")));
                }
                self.at += 1;
                return Ok(Element {
                    name,
                    attributes,
                    tail,
                    body: Some(Body { nodes, close_tail }),
                });
            }
            if rest.is_empty() {
                return Err(self.fault(format!("<{name}> is never closed")));
            }
            let other = if rest.starts_with("<!--") {
                self.through("-->", "a comment")?
            } else if rest.starts_with("<![CDATA[") {
                self.through("]]>", "a CDATA section")?
            } else if rest.starts_with("<?") {
                self.through("?>", "a processing instruction")?
            } else if rest.starts_with('<') {
                nodes.push(Node::Element(self.element(depth + 1)?));
                continue;
            } else {
                self.run(|c| c != '<')
            };
            nodes.push(Node::Other(other.to_string()));
        }
    }
}

/// Whether `c` may stand in the name of an element or an attribute.
fn is_name(c: char) -> bool {
    !(c.is_whitespace() || matches!(c, '<' | '>' | '/' | '=' | '"' | '\''))
}
