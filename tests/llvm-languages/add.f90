integer(c_int) function lume_test_add_fortran(a, b) bind(C, name="lume_test_add_fortran")
    use iso_c_binding, only: c_int
    integer(c_int), value :: a, b
    lume_test_add_fortran = a + b
end function lume_test_add_fortran
