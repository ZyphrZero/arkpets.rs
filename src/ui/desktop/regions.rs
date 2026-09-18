use eframe::egui::{Pos2, Rect, Vec2};

const EPSILON: f32 = 0.001;

/// Cover every vertical strip, so gaps between staggered displays stay solid.
pub fn covered(rect: Rect, areas: &[Rect]) -> bool {
    if areas.iter().any(|area| area.contains_rect(rect)) {
        return true;
    }
    let mut xs = vec![rect.left(), rect.right()];
    for area in areas {
        for x in [area.left(), area.right()] {
            if x > rect.left() && x < rect.right() {
                xs.push(x);
            }
        }
    }
    xs.sort_by(f32::total_cmp);
    xs.windows(2).all(|strip| {
        let x = (strip[0] + strip[1]) * 0.5;
        let mut intervals: Vec<_> = areas
            .iter()
            .filter(|area| x >= area.left() && x <= area.right())
            .map(|area| (area.top(), area.bottom()))
            .collect();
        intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut bottom = rect.top();
        for (start, end) in intervals {
            if start > bottom + EPSILON {
                break;
            }
            bottom = bottom.max(end);
        }
        bottom + EPSILON >= rect.bottom()
    })
}

/// Find the closest valid placement, used for deployment, dragging and hotplug.
pub fn project(position: Pos2, size: Vec2, areas: &[Rect]) -> Option<Pos2> {
    if covered(Rect::from_min_size(position, size), areas) {
        return Some(position);
    }
    let mut xs = vec![position.x];
    let mut ys = vec![position.y];
    for area in areas {
        for edge in [area.left(), area.right()] {
            xs.extend([edge, edge - size.x]);
        }
        for edge in [area.top(), area.bottom()] {
            ys.extend([edge, edge - size.y]);
        }
    }
    xs.iter()
        .flat_map(|&x| ys.iter().map(move |&y| Pos2::new(x, y)))
        .filter(|&p| covered(Rect::from_min_size(p, size), areas))
        .min_by(|a, b| a.distance_sq(position).total_cmp(&b.distance_sq(position)))
}

/// Sweep each axis through the union; even a fast throw cannot jump a gap.
pub fn travel(from: Pos2, to: Pos2, size: Vec2, areas: &[Rect]) -> Pos2 {
    fn axis(from: Pos2, to: Pos2, size: Vec2, areas: &[Rect]) -> Pos2 {
        let valid = |p| {
            covered(
                Rect::from_min_size(from, size).union(Rect::from_min_size(p, size)),
                areas,
            )
        };
        if valid(to) {
            return to;
        }
        let (mut low, mut high) = (0.0, 1.0);
        for _ in 0..24 {
            let middle = (low + high) * 0.5;
            if valid(from + (to - from) * middle) {
                low = middle;
            } else {
                high = middle;
            }
        }
        from + (to - from) * low
    }
    let horizontal = axis(from, Pos2::new(to.x, from.y), size, areas);
    axis(horizontal, Pos2::new(horizontal.x, to.y), size, areas)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn area(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h))
    }

    #[test]
    fn adjacent_displays_allow_seam_crossing_but_staggered_gaps_do_not() {
        let size = Vec2::new(100.0, 100.0);
        let areas = [
            area(-1000.0, 0.0, 1000.0, 800.0),
            area(0.0, 200.0, 1200.0, 800.0),
        ];
        assert_eq!(
            travel(
                Pos2::new(-150.0, 400.0),
                Pos2::new(50.0, 400.0),
                size,
                &areas
            ),
            Pos2::new(50.0, 400.0)
        );
        let blocked = travel(Pos2::new(-150.0, 0.0), Pos2::new(50.0, 0.0), size, &areas);
        assert!((blocked.x + 100.0).abs() < 0.01);
        assert!(covered(Rect::from_min_size(blocked, size), &areas));
        let gap = [area(0.0, 0.0, 500.0, 500.0), area(600.0, 0.0, 500.0, 500.0)];
        assert!(travel(Pos2::new(350.0, 300.0), Pos2::new(800.0, 300.0), size, &gap).x < 401.0);
    }

    #[test]
    fn projection_recovers_removed_displays_and_never_selects_empty_space() {
        let areas = [
            area(-800.0, -600.0, 800.0, 600.0),
            area(0.0, 0.0, 1200.0, 800.0),
        ];
        let size = Vec2::new(200.0, 150.0);
        for p in [
            Pos2::new(-400.0, 200.0),
            Pos2::new(100.0, -300.0),
            Pos2::new(-2000.0, -1000.0),
        ] {
            let projected = project(p, size, &areas).unwrap();
            assert!(covered(Rect::from_min_size(projected, size), &areas));
            assert!(covered(
                Rect::from_min_size(project(projected, size, &areas[1..]).unwrap(), size),
                &areas[1..]
            ));
        }
        assert!(project(Pos2::ZERO, Vec2::splat(4000.0), &areas).is_none());
    }
}
