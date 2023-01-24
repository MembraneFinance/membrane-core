(function() {var implementors = {
"der":[["impl&lt;'a&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"der/asn1/struct.BitStringIter.html\" title=\"struct der::asn1::BitStringIter\">BitStringIter</a>&lt;'a&gt;"]],
"either":[["impl&lt;L, R&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"enum\" href=\"either/enum.Either.html\" title=\"enum either::Either\">Either</a>&lt;L, R&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;L: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;R: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a>&lt;Item = L::<a class=\"associatedtype\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html#associatedtype.Item\" title=\"type core::iter::traits::iterator::Iterator::Item\">Item</a>&gt;,</span>"]],
"generic_array":[["impl&lt;T, N&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"generic_array/iter/struct.GenericArrayIter.html\" title=\"struct generic_array::iter::GenericArrayIter\">GenericArrayIter</a>&lt;T, N&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;N: <a class=\"trait\" href=\"generic_array/trait.ArrayLength.html\" title=\"trait generic_array::ArrayLength\">ArrayLength</a>&lt;T&gt;,</span>"]],
"hashbrown":[["impl&lt;K, V, A:&nbsp;Allocator + <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"hashbrown/hash_map/struct.IntoKeys.html\" title=\"struct hashbrown::hash_map::IntoKeys\">IntoKeys</a>&lt;K, V, A&gt;"],["impl&lt;K, V, A:&nbsp;Allocator + <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"hashbrown/hash_map/struct.IntoValues.html\" title=\"struct hashbrown::hash_map::IntoValues\">IntoValues</a>&lt;K, V, A&gt;"],["impl&lt;K, V, F&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"hashbrown/hash_map/struct.DrainFilter.html\" title=\"struct hashbrown::hash_map::DrainFilter\">DrainFilter</a>&lt;'_, K, V, F&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;F: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/ops/function/trait.FnMut.html\" title=\"trait core::ops::function::FnMut\">FnMut</a>(<a class=\"primitive\" href=\"https://doc.rust-lang.org/1.66.0/std/primitive.reference.html\">&amp;</a>K, <a class=\"primitive\" href=\"https://doc.rust-lang.org/1.66.0/std/primitive.reference.html\">&amp;mut </a>V) -&gt; <a class=\"primitive\" href=\"https://doc.rust-lang.org/1.66.0/std/primitive.bool.html\">bool</a>,</span>"],["impl&lt;K, V&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"hashbrown/hash_map/struct.Iter.html\" title=\"struct hashbrown::hash_map::Iter\">Iter</a>&lt;'_, K, V&gt;"],["impl&lt;K, V&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"hashbrown/hash_map/struct.IterMut.html\" title=\"struct hashbrown::hash_map::IterMut\">IterMut</a>&lt;'_, K, V&gt;"],["impl&lt;K, V, A:&nbsp;Allocator + <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"hashbrown/hash_map/struct.IntoIter.html\" title=\"struct hashbrown::hash_map::IntoIter\">IntoIter</a>&lt;K, V, A&gt;"],["impl&lt;K, V&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"hashbrown/hash_map/struct.Keys.html\" title=\"struct hashbrown::hash_map::Keys\">Keys</a>&lt;'_, K, V&gt;"],["impl&lt;K, V&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"hashbrown/hash_map/struct.Values.html\" title=\"struct hashbrown::hash_map::Values\">Values</a>&lt;'_, K, V&gt;"],["impl&lt;K, V&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"hashbrown/hash_map/struct.ValuesMut.html\" title=\"struct hashbrown::hash_map::ValuesMut\">ValuesMut</a>&lt;'_, K, V&gt;"],["impl&lt;K, V, A:&nbsp;Allocator + <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"hashbrown/hash_map/struct.Drain.html\" title=\"struct hashbrown::hash_map::Drain\">Drain</a>&lt;'_, K, V, A&gt;"],["impl&lt;K&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"hashbrown/hash_set/struct.Iter.html\" title=\"struct hashbrown::hash_set::Iter\">Iter</a>&lt;'_, K&gt;"],["impl&lt;K, A:&nbsp;Allocator + <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"hashbrown/hash_set/struct.IntoIter.html\" title=\"struct hashbrown::hash_set::IntoIter\">IntoIter</a>&lt;K, A&gt;"],["impl&lt;K, A:&nbsp;Allocator + <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"hashbrown/hash_set/struct.Drain.html\" title=\"struct hashbrown::hash_set::Drain\">Drain</a>&lt;'_, K, A&gt;"],["impl&lt;K, F, A:&nbsp;Allocator + <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"hashbrown/hash_set/struct.DrainFilter.html\" title=\"struct hashbrown::hash_set::DrainFilter\">DrainFilter</a>&lt;'_, K, F, A&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;F: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/ops/function/trait.FnMut.html\" title=\"trait core::ops::function::FnMut\">FnMut</a>(<a class=\"primitive\" href=\"https://doc.rust-lang.org/1.66.0/std/primitive.reference.html\">&amp;</a>K) -&gt; <a class=\"primitive\" href=\"https://doc.rust-lang.org/1.66.0/std/primitive.bool.html\">bool</a>,</span>"],["impl&lt;T, S, A&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"hashbrown/hash_set/struct.Intersection.html\" title=\"struct hashbrown::hash_set::Intersection\">Intersection</a>&lt;'_, T, S, A&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;T: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/cmp/trait.Eq.html\" title=\"trait core::cmp::Eq\">Eq</a> + <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/hash/trait.Hash.html\" title=\"trait core::hash::Hash\">Hash</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;S: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/hash/trait.BuildHasher.html\" title=\"trait core::hash::BuildHasher\">BuildHasher</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;A: Allocator + <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a>,</span>"],["impl&lt;T, S, A&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"hashbrown/hash_set/struct.Difference.html\" title=\"struct hashbrown::hash_set::Difference\">Difference</a>&lt;'_, T, S, A&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;T: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/cmp/trait.Eq.html\" title=\"trait core::cmp::Eq\">Eq</a> + <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/hash/trait.Hash.html\" title=\"trait core::hash::Hash\">Hash</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;S: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/hash/trait.BuildHasher.html\" title=\"trait core::hash::BuildHasher\">BuildHasher</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;A: Allocator + <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a>,</span>"],["impl&lt;T, S, A&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"hashbrown/hash_set/struct.SymmetricDifference.html\" title=\"struct hashbrown::hash_set::SymmetricDifference\">SymmetricDifference</a>&lt;'_, T, S, A&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;T: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/cmp/trait.Eq.html\" title=\"trait core::cmp::Eq\">Eq</a> + <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/hash/trait.Hash.html\" title=\"trait core::hash::Hash\">Hash</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;S: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/hash/trait.BuildHasher.html\" title=\"trait core::hash::BuildHasher\">BuildHasher</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;A: Allocator + <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a>,</span>"],["impl&lt;T, S, A&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"hashbrown/hash_set/struct.Union.html\" title=\"struct hashbrown::hash_set::Union\">Union</a>&lt;'_, T, S, A&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;T: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/cmp/trait.Eq.html\" title=\"trait core::cmp::Eq\">Eq</a> + <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/hash/trait.Hash.html\" title=\"trait core::hash::Hash\">Hash</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;S: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/hash/trait.BuildHasher.html\" title=\"trait core::hash::BuildHasher\">BuildHasher</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;A: Allocator + <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a>,</span>"]],
"itertools":[["impl&lt;I, J&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.Interleave.html\" title=\"struct itertools::structs::Interleave\">Interleave</a>&lt;I, J&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;I: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html\" title=\"trait core::iter::traits::iterator::Iterator\">Iterator</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;J: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html\" title=\"trait core::iter::traits::iterator::Iterator\">Iterator</a>&lt;Item = I::<a class=\"associatedtype\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html#associatedtype.Item\" title=\"type core::iter::traits::iterator::Iterator::Item\">Item</a>&gt;,</span>"],["impl&lt;I, J&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.InterleaveShortest.html\" title=\"struct itertools::structs::InterleaveShortest\">InterleaveShortest</a>&lt;I, J&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;I: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;J: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a>&lt;Item = I::<a class=\"associatedtype\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html#associatedtype.Item\" title=\"type core::iter::traits::iterator::Iterator::Item\">Item</a>&gt;,</span>"],["impl&lt;I, J&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.Product.html\" title=\"struct itertools::structs::Product\">Product</a>&lt;I, J&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;I: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;J: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a> + <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;I::<a class=\"associatedtype\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html#associatedtype.Item\" title=\"type core::iter::traits::iterator::Iterator::Item\">Item</a>: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a>,</span>"],["impl&lt;I, J, F&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.MergeBy.html\" title=\"struct itertools::structs::MergeBy\">MergeBy</a>&lt;I, J, F&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;I: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;J: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a>&lt;Item = I::<a class=\"associatedtype\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html#associatedtype.Item\" title=\"type core::iter::traits::iterator::Iterator::Item\">Item</a>&gt;,<br>&nbsp;&nbsp;&nbsp;&nbsp;F: MergePredicate&lt;I::<a class=\"associatedtype\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html#associatedtype.Item\" title=\"type core::iter::traits::iterator::Iterator::Item\">Item</a>&gt;,</span>"],["impl&lt;I, T&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.TupleCombinations.html\" title=\"struct itertools::structs::TupleCombinations\">TupleCombinations</a>&lt;I, T&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;I: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;T: HasCombination&lt;I&gt;,</span>"],["impl&lt;I, F, T, E&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.FilterOk.html\" title=\"struct itertools::structs::FilterOk\">FilterOk</a>&lt;I, F&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;I: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a>&lt;Item = <a class=\"enum\" href=\"https://doc.rust-lang.org/1.66.0/core/result/enum.Result.html\" title=\"enum core::result::Result\">Result</a>&lt;T, E&gt;&gt;,<br>&nbsp;&nbsp;&nbsp;&nbsp;F: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/ops/function/trait.FnMut.html\" title=\"trait core::ops::function::FnMut\">FnMut</a>(<a class=\"primitive\" href=\"https://doc.rust-lang.org/1.66.0/std/primitive.reference.html\">&amp;</a>T) -&gt; <a class=\"primitive\" href=\"https://doc.rust-lang.org/1.66.0/std/primitive.bool.html\">bool</a>,</span>"],["impl&lt;I, F, T, U, E&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.FilterMapOk.html\" title=\"struct itertools::structs::FilterMapOk\">FilterMapOk</a>&lt;I, F&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;I: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a>&lt;Item = <a class=\"enum\" href=\"https://doc.rust-lang.org/1.66.0/core/result/enum.Result.html\" title=\"enum core::result::Result\">Result</a>&lt;T, E&gt;&gt;,<br>&nbsp;&nbsp;&nbsp;&nbsp;F: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/ops/function/trait.FnMut.html\" title=\"trait core::ops::function::FnMut\">FnMut</a>(T) -&gt; <a class=\"enum\" href=\"https://doc.rust-lang.org/1.66.0/core/option/enum.Option.html\" title=\"enum core::option::Option\">Option</a>&lt;U&gt;,</span>"],["impl&lt;I, F&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.Positions.html\" title=\"struct itertools::structs::Positions\">Positions</a>&lt;I, F&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;I: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;F: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/ops/function/trait.FnMut.html\" title=\"trait core::ops::function::FnMut\">FnMut</a>(I::<a class=\"associatedtype\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html#associatedtype.Item\" title=\"type core::iter::traits::iterator::Iterator::Item\">Item</a>) -&gt; <a class=\"primitive\" href=\"https://doc.rust-lang.org/1.66.0/std/primitive.bool.html\">bool</a>,</span>"],["impl&lt;I, F&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.Update.html\" title=\"struct itertools::structs::Update\">Update</a>&lt;I, F&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;I: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;F: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/ops/function/trait.FnMut.html\" title=\"trait core::ops::function::FnMut\">FnMut</a>(&amp;mut I::<a class=\"associatedtype\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html#associatedtype.Item\" title=\"type core::iter::traits::iterator::Iterator::Item\">Item</a>),</span>"],["impl&lt;I&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.Combinations.html\" title=\"struct itertools::structs::Combinations\">Combinations</a>&lt;I&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;I: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html\" title=\"trait core::iter::traits::iterator::Iterator\">Iterator</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;I::<a class=\"associatedtype\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html#associatedtype.Item\" title=\"type core::iter::traits::iterator::Iterator::Item\">Item</a>: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a>,</span>"],["impl&lt;I&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.CombinationsWithReplacement.html\" title=\"struct itertools::structs::CombinationsWithReplacement\">CombinationsWithReplacement</a>&lt;I&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;I: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html\" title=\"trait core::iter::traits::iterator::Iterator\">Iterator</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;I::<a class=\"associatedtype\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html#associatedtype.Item\" title=\"type core::iter::traits::iterator::Iterator::Item\">Item</a>: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a>,</span>"],["impl&lt;I, T, E&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.FlattenOk.html\" title=\"struct itertools::structs::FlattenOk\">FlattenOk</a>&lt;I, T, E&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;I: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a>&lt;Item = <a class=\"enum\" href=\"https://doc.rust-lang.org/1.66.0/core/result/enum.Result.html\" title=\"enum core::result::Result\">Result</a>&lt;T, E&gt;&gt;,<br>&nbsp;&nbsp;&nbsp;&nbsp;T: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/collect/trait.IntoIterator.html\" title=\"trait core::iter::traits::collect::IntoIterator\">IntoIterator</a>,</span>"],["impl&lt;I, ElemF&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.IntersperseWith.html\" title=\"struct itertools::structs::IntersperseWith\">IntersperseWith</a>&lt;I, ElemF&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;I: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html\" title=\"trait core::iter::traits::iterator::Iterator\">Iterator</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;ElemF: IntersperseElement&lt;I::<a class=\"associatedtype\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html#associatedtype.Item\" title=\"type core::iter::traits::iterator::Iterator::Item\">Item</a>&gt;,</span>"],["impl&lt;I, F&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.KMergeBy.html\" title=\"struct itertools::structs::KMergeBy\">KMergeBy</a>&lt;I, F&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;I: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html\" title=\"trait core::iter::traits::iterator::Iterator\">Iterator</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;F: KMergePredicate&lt;I::<a class=\"associatedtype\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html#associatedtype.Item\" title=\"type core::iter::traits::iterator::Iterator::Item\">Item</a>&gt;,</span>"],["impl&lt;I, F&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.PadUsing.html\" title=\"struct itertools::structs::PadUsing\">PadUsing</a>&lt;I, F&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;I: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;F: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/ops/function/trait.FnMut.html\" title=\"trait core::ops::function::FnMut\">FnMut</a>(<a class=\"primitive\" href=\"https://doc.rust-lang.org/1.66.0/std/primitive.usize.html\">usize</a>) -&gt; I::<a class=\"associatedtype\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html#associatedtype.Item\" title=\"type core::iter::traits::iterator::Iterator::Item\">Item</a>,</span>"],["impl&lt;I&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.Powerset.html\" title=\"struct itertools::structs::Powerset\">Powerset</a>&lt;I&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;I: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html\" title=\"trait core::iter::traits::iterator::Iterator\">Iterator</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;I::<a class=\"associatedtype\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html#associatedtype.Item\" title=\"type core::iter::traits::iterator::Iterator::Item\">Item</a>: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a>,</span>"],["impl&lt;A, I&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.RcIter.html\" title=\"struct itertools::structs::RcIter\">RcIter</a>&lt;I&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;I: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a>&lt;Item = A&gt;,</span>"],["impl&lt;A&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.RepeatN.html\" title=\"struct itertools::structs::RepeatN\">RepeatN</a>&lt;A&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;A: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a>,</span>"],["impl&lt;I, T&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.TupleWindows.html\" title=\"struct itertools::structs::TupleWindows\">TupleWindows</a>&lt;I, T&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;I: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a>&lt;Item = T::Item&gt;,<br>&nbsp;&nbsp;&nbsp;&nbsp;T: <a class=\"trait\" href=\"itertools/traits/trait.HomogeneousTuple.html\" title=\"trait itertools::traits::HomogeneousTuple\">HomogeneousTuple</a> + <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;T::Item: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a>,</span>"],["impl&lt;I, V, F&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.UniqueBy.html\" title=\"struct itertools::structs::UniqueBy\">UniqueBy</a>&lt;I, V, F&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;I: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;V: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/cmp/trait.Eq.html\" title=\"trait core::cmp::Eq\">Eq</a> + <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/hash/trait.Hash.html\" title=\"trait core::hash::Hash\">Hash</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;F: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/ops/function/trait.FnMut.html\" title=\"trait core::ops::function::FnMut\">FnMut</a>(&amp;I::<a class=\"associatedtype\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html#associatedtype.Item\" title=\"type core::iter::traits::iterator::Iterator::Item\">Item</a>) -&gt; V,</span>"],["impl&lt;I&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.Unique.html\" title=\"struct itertools::structs::Unique\">Unique</a>&lt;I&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;I: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;I::<a class=\"associatedtype\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html#associatedtype.Item\" title=\"type core::iter::traits::iterator::Iterator::Item\">Item</a>: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/cmp/trait.Eq.html\" title=\"trait core::cmp::Eq\">Eq</a> + <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/hash/trait.Hash.html\" title=\"trait core::hash::Hash\">Hash</a> + <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a>,</span>"],["impl&lt;I:&nbsp;<a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html\" title=\"trait core::iter::traits::iterator::Iterator\">Iterator</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.WithPosition.html\" title=\"struct itertools::structs::WithPosition\">WithPosition</a>&lt;I&gt;"],["impl&lt;T, U&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"itertools/structs/struct.ZipLongest.html\" title=\"struct itertools::structs::ZipLongest\">ZipLongest</a>&lt;T, U&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;T: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html\" title=\"trait core::iter::traits::iterator::Iterator\">Iterator</a>,<br>&nbsp;&nbsp;&nbsp;&nbsp;U: <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/iterator/trait.Iterator.html\" title=\"trait core::iter::traits::iterator::Iterator\">Iterator</a>,</span>"]],
"serde_json":[["impl&lt;'de, R, T&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"serde_json/struct.StreamDeserializer.html\" title=\"struct serde_json::StreamDeserializer\">StreamDeserializer</a>&lt;'de, R, T&gt;<span class=\"where fmt-newline\">where<br>&nbsp;&nbsp;&nbsp;&nbsp;R: <a class=\"trait\" href=\"serde_json/de/trait.Read.html\" title=\"trait serde_json::de::Read\">Read</a>&lt;'de&gt; + Fused,<br>&nbsp;&nbsp;&nbsp;&nbsp;T: <a class=\"trait\" href=\"serde/de/trait.Deserialize.html\" title=\"trait serde::de::Deserialize\">Deserialize</a>&lt;'de&gt;,</span>"],["impl&lt;'a&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"serde_json/map/struct.Iter.html\" title=\"struct serde_json::map::Iter\">Iter</a>&lt;'a&gt;"],["impl&lt;'a&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"serde_json/map/struct.IterMut.html\" title=\"struct serde_json::map::IterMut\">IterMut</a>&lt;'a&gt;"],["impl <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"serde_json/map/struct.IntoIter.html\" title=\"struct serde_json::map::IntoIter\">IntoIter</a>"],["impl&lt;'a&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"serde_json/map/struct.Keys.html\" title=\"struct serde_json::map::Keys\">Keys</a>&lt;'a&gt;"],["impl&lt;'a&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"serde_json/map/struct.Values.html\" title=\"struct serde_json::map::Values\">Values</a>&lt;'a&gt;"],["impl&lt;'a&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.66.0/core/iter/traits/marker/trait.FusedIterator.html\" title=\"trait core::iter::traits::marker::FusedIterator\">FusedIterator</a> for <a class=\"struct\" href=\"serde_json/map/struct.ValuesMut.html\" title=\"struct serde_json::map::ValuesMut\">ValuesMut</a>&lt;'a&gt;"]]
};if (window.register_implementors) {window.register_implementors(implementors);} else {window.pending_implementors = implementors;}})()