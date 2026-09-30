#let hbar = sym.planck
#let homework(course: "", title: "", author: none, body) = {
  set page(margin: 1in, numbering: "1")
  set math.equation(numbering: "(1)")
  show heading.where(level: 1): set text(size: 14pt)
  align(center)[*#course: #title* \ #emph(author)]
  body
}
#show: homework.with(course: "ECE 69500 DS", title: "Homework 3", author: "A. Student")

= Problem 1

Consider the *quantum harmonic oscillator* with Hamiltonian
$H = p^2 / (2m) + 1/2 m omega^2 x^2$. We use _natural units_ only at the end.

== Part 1(a)

Define the ladder operators $hat(a) = sqrt((m omega)/(2 hbar)) (x + i hat(p)/(m omega))$
and its adjoint $hat(a)^dagger$. Show that they satisfy
$ [hat(a), hat(a)^dagger] = 1 $ <commutator>

Using @commutator, the Hamiltonian becomes
$ hat(H) &= hbar omega (hat(a)^dagger hat(a) + 1/2) \
         &= sum_(n=0)^oo E_n |n chevron.r chevron.l n| . $ <spectrum>

== Part 1(b)

The ground state obeys $hat(a) |0 chevron.r = 0$, so
$ psi_0 (x) = ((m omega)/(pi hbar))^(1\/4) exp(-(m omega x^2)/(2 hbar)) $

- Normalize: $integral_(-oo)^oo |psi_0|^2 dif x = 1$
- Check $E_0 = hbar omega \/ 2 >= 0$ and $n arrow.r oo$, $x lt.eq 0$ limits
+ Energies are evenly spaced, $Delta E = hbar omega$.
+ Degeneracy is $g_n = 1$ for all $n in NN$.

/ Ladder: an operator mapping $|n chevron.r$ to $|n plus.minus 1 chevron.r$.

= Problem 2

#let k = 3
For $k = #k$ the matrix $A = mat(1, 2; 3, 4)$ has $det A != 0$, and
$ vec(x_1, x_2) = A^(-1) vec(b_1, b_2), quad "where" b in RR^#k $

Numerically (see @spectrum and #link("https://typst.app/docs")[the docs]):

```python
import numpy as np
omega, hbar = 1.0, 1.0
print([hbar * omega * (n + 0.5) for n in range(4)])
```

Inline code like `np.linalg.eig` works too. Escapes: \#, \$, \*.
/* Block comments /* nest */ too. */
