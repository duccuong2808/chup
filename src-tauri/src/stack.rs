//! Stack tối đa 5 shot (SPEC 5.4). Thứ tự: cũ nhất trước, mới nhất cuối
//! (frontend vẽ mới nhất sát góc dưới).

use std::collections::VecDeque;

use crate::capture::Shot;

pub const MAX_STACK: usize = 5;

#[derive(Default)]
pub struct Stack {
    items: VecDeque<Shot>,
}

impl Stack {
    /// Thêm shot; trả về các shot bị đẩy ra (chỉ khỏi stack, file gốc vẫn giữ).
    pub fn push(&mut self, shot: Shot) -> Vec<Shot> {
        self.items.push_back(shot);
        let mut evicted = Vec::new();
        while self.items.len() > MAX_STACK {
            if let Some(s) = self.items.pop_front() {
                evicted.push(s);
            }
        }
        evicted
    }

    pub fn remove(&mut self, id: &str) -> Option<Shot> {
        let idx = self.items.iter().position(|s| s.id == id)?;
        self.items.remove(idx)
    }

    pub fn clear(&mut self) -> Vec<Shot> {
        self.items.drain(..).collect()
    }

    pub fn get(&self, id: &str) -> Option<&Shot> {
        self.items.iter().find(|s| s.id == id)
    }

    pub fn list(&self) -> Vec<Shot> {
        self.items.iter().cloned().collect()
    }
}

/// Xoá thumbnail trong cache của các shot đã rời stack.
pub fn drop_thumbnails(shots: &[Shot]) {
    for s in shots {
        let _ = std::fs::remove_file(&s.thumb_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn shot(id: &str) -> Shot {
        Shot {
            id: id.into(),
            path: PathBuf::from(format!("{id}.png")),
            thumb_path: PathBuf::from(format!("t-{id}.png")),
            thumb_data_url: String::new(),
            width: 1,
            height: 1,
            created_at: chrono::Local::now(),
        }
    }

    #[test]
    fn keeps_five_newest() {
        let mut s = Stack::default();
        let mut evicted = vec![];
        for i in 0..7 {
            evicted.extend(s.push(shot(&i.to_string())));
        }
        let ids: Vec<_> = s.list().into_iter().map(|x| x.id).collect();
        assert_eq!(ids, ["2", "3", "4", "5", "6"]);
        let ev: Vec<_> = evicted.into_iter().map(|x| x.id).collect();
        assert_eq!(ev, ["0", "1"]);
    }

    #[test]
    fn remove_and_clear() {
        let mut s = Stack::default();
        s.push(shot("a"));
        s.push(shot("b"));
        assert_eq!(s.remove("a").unwrap().id, "a");
        assert!(s.remove("a").is_none());
        assert!(s.get("b").is_some());
        assert_eq!(s.clear().len(), 1);
        assert!(s.list().is_empty());
    }
}
