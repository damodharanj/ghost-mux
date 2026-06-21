#[derive(Clone, Copy, Debug)]
pub enum SplitDir {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug)]
pub enum PanelLayout {
    Leaf(usize),   
    HSplit {
        left: Box<PanelLayout>,
        right: Box<PanelLayout>,
        id: usize,
    },
    VSplit {
        top: Box<PanelLayout>,
        bot: Box<PanelLayout>,
        id: usize
    },
}

impl PanelLayout {
    /// Collect the IDs of all split nodes (HSplit / VSplit) in this tree.
    pub fn collect_split_ids(&self) -> Vec<usize> {
        let mut ids = vec![];
        self.collect_split_ids_into(&mut ids);
        ids
    }

    fn collect_split_ids_into(&self, out: &mut Vec<usize>) {
        match self {
            PanelLayout::Leaf(_) => {}
            PanelLayout::HSplit { left, right, id } => {
                out.push(*id);
                left.collect_split_ids_into(out);
                right.collect_split_ids_into(out);
            }
            PanelLayout::VSplit { top, bot, id } => {
                out.push(*id);
                top.collect_split_ids_into(out);
                bot.collect_split_ids_into(out);
            }
        }
    }

    pub fn leaf_count(&self) -> usize {
        match self {
            PanelLayout::Leaf(_) => 1,
            PanelLayout::HSplit { left, right, .. } => left.leaf_count() + right.leaf_count(),
            PanelLayout::VSplit { top, bot, .. } => top.leaf_count() + bot.leaf_count(),
        }
    }

    pub fn split(self, target: usize, dir: SplitDir, new_panel: usize, new_split: usize) -> Self {
        match self {
            PanelLayout::Leaf(id) if id == target => {
                let a = Box::new(PanelLayout::Leaf(id));
                let b = Box::new(PanelLayout::Leaf(new_panel));
                match dir {
                    SplitDir::Horizontal => PanelLayout::HSplit {
                        left: a,
                        right: b,
                        id: new_split,
                    },
                    SplitDir::Vertical => PanelLayout::VSplit {
                        top: a,
                        bot: b,
                        id: new_split,
                    },
                }
            }
            PanelLayout::Leaf(id) => PanelLayout::Leaf(id),
            PanelLayout::HSplit { left, right, id } => PanelLayout::HSplit {
                left: Box::new(left.split(target, dir, new_panel, new_split)),
                right: Box::new(right.split(target, dir, new_panel, new_split)),
                id,
            },
            PanelLayout::VSplit { top, bot, id } => PanelLayout::VSplit {
                top: Box::new(top.split(target, dir, new_panel, new_split)),
                bot: Box::new(bot.split(target, dir, new_panel, new_split)),
                id,
            },
        }
    }

    pub fn close(self, target: usize) -> Option<Self> {
        match self {
            PanelLayout::Leaf(id) => {
                if id == target {
                    None
                } else {
                    Some(PanelLayout::Leaf(id))
                }
            }
            PanelLayout::HSplit { left, right, id } => {
                match (left.close(target), right.close(target)) {
                    (None, Some(r)) => Some(r),
                    (Some(l), None) => Some(l),
                    (Some(l), Some(r)) => Some(PanelLayout::HSplit {
                        left: Box::new(l),
                        right: Box::new(r),
                        id,
                    }),
                    (None, None) => None,
                }
            }
            PanelLayout::VSplit { top, bot, id } => match (top.close(target), bot.close(target)) {
                (None, Some(b)) => Some(b),
                (Some(t), None) => Some(t),
                (Some(t), Some(b)) => Some(PanelLayout::VSplit {
                    top: Box::new(t),
                    bot: Box::new(b),
                    id,
                }),
                (None, None) => None,
            },
        }
    }
}

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub enum PanelContent {
    Terminal,
    FileExplorer,
    Git,
    Browser {
        url: String,
    },
    Editor {
        path: PathBuf,
        is_diff: bool,
        status: Option<String>,
    },
    Diagnostics,
}
